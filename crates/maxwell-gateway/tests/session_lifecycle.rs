//! Integration tests for gateway session lifecycle management.

use maxwell_gateway::{
    ClientRequest, Gateway, GatewayConfig, GatewayError, SessionId, SessionState,
};

use maxwell_publisher::{EventKind, PublisherError, SubscriptionFilter};

type TestGateway = Gateway<u32, u64>;

fn gateway() -> TestGateway {
    Gateway::new(GatewayConfig {
        heartbeat_interval_ns: 50,
        session_timeout_ns: 150,
        ..GatewayConfig::default()
    })
    .unwrap()
}

fn subscribe(gateway: &mut TestGateway, id: SessionId, now_ns: u64) {
    gateway
        .handle_request(
            id,
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            now_ns,
        )
        .unwrap();
}

#[test]
fn complete_session_lifecycle() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();

    assert_eq!(
        gateway.session(id).unwrap().state(),
        SessionState::Connected
    );

    subscribe(&mut gateway, id, 110);

    assert_eq!(
        gateway.session(id).unwrap().state(),
        SessionState::AwaitingSnapshot
    );

    gateway.activate_snapshot(id, 0, 120).unwrap();

    assert_eq!(gateway.session(id).unwrap().state(), SessionState::Active);

    gateway
        .handle_request(id, ClientRequest::Unsubscribe, 130)
        .unwrap();

    assert_eq!(
        gateway.session(id).unwrap().state(),
        SessionState::Connected
    );

    gateway
        .handle_request(id, ClientRequest::Disconnect, 140)
        .unwrap();

    assert_eq!(gateway.session_count(), 0);

    let stats = gateway.stats();

    assert_eq!(stats.sessions_connected, 1);
    assert_eq!(stats.sessions_disconnected, 1);
    assert_eq!(stats.subscriptions_created, 1);
    assert_eq!(stats.subscriptions_removed, 1);
    assert_eq!(stats.snapshots_activated, 1);
}

#[test]
fn rejects_snapshot_without_subscription() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();

    assert_eq!(
        gateway.activate_snapshot(id, 0, 100),
        Err(GatewayError::InvalidSessionState { session_id: id })
    );
}

#[test]
fn rejects_stale_snapshot_watermark() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();
    subscribe(&mut gateway, id, 100);

    gateway
        .publish(Some(100), EventKind::Trade, 1000, 42)
        .unwrap();

    assert_eq!(
        gateway.activate_snapshot(id, 0, 110),
        Err(GatewayError::Publisher(PublisherError::InvalidWatermark {
            expected: 1,
            received: 0,
        },))
    );

    assert_eq!(
        gateway.session(id).unwrap().state(),
        SessionState::AwaitingSnapshot
    );

    gateway.activate_snapshot(id, 1, 110).unwrap();

    assert!(gateway.session(id).unwrap().is_active());
}

#[test]
fn duplicate_subscription_is_rejected() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();
    subscribe(&mut gateway, id, 100);

    let result = gateway.handle_request(
        id,
        ClientRequest::Subscribe {
            filter: SubscriptionFilter::all(),
        },
        110,
    );

    assert_eq!(
        result,
        Err(GatewayError::InvalidSessionState { session_id: id })
    );

    assert_eq!(gateway.stats().subscriptions_created, 1);
    assert_eq!(gateway.stats().client_requests_rejected, 1);
}

#[test]
fn unsubscribe_discards_pending_publications() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();
    subscribe(&mut gateway, id, 100);
    gateway.activate_snapshot(id, 0, 100).unwrap();

    gateway
        .publish(Some(100), EventKind::Trade, 1000, 42)
        .unwrap();

    assert_eq!(gateway.publisher_stats().pending_deliveries(), 1);

    gateway
        .handle_request(id, ClientRequest::Unsubscribe, 110)
        .unwrap();

    assert_eq!(gateway.publisher_stats().pending_deliveries(), 0);

    assert_eq!(gateway.publisher_stats().deliveries_discarded, 1);
}

#[test]
fn expires_idle_session() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();

    assert_eq!(gateway.expire_sessions(249).unwrap(), 0);
    assert_eq!(gateway.expire_sessions(250).unwrap(), 1);

    assert_eq!(gateway.session(id).unwrap().state(), SessionState::Expired);

    assert_eq!(gateway.stats().sessions_expired, 1);

    // Repeated expiration scans are idempotent.
    assert_eq!(gateway.expire_sessions(300).unwrap(), 0);

    gateway.disconnect(id).unwrap();

    assert_eq!(gateway.session_count(), 0);
}

#[test]
fn heartbeat_extends_session_lifetime() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();

    gateway
        .handle_request(id, ClientRequest::Heartbeat, 200)
        .unwrap();

    assert_eq!(gateway.expire_sessions(250).unwrap(), 0);
    assert_eq!(gateway.expire_sessions(349).unwrap(), 0);
    assert_eq!(gateway.expire_sessions(350).unwrap(), 1);

    assert_eq!(gateway.stats().heartbeats_received, 1);
}

#[test]
fn late_heartbeat_cannot_revive_expired_session() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();

    assert_eq!(
        gateway.handle_request(id, ClientRequest::Heartbeat, 250,),
        Err(GatewayError::SessionExpired { session_id: id })
    );

    assert_eq!(gateway.session(id).unwrap().state(), SessionState::Expired);
}

#[test]
fn expiration_releases_publisher_resources() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();
    subscribe(&mut gateway, id, 100);
    gateway.activate_snapshot(id, 0, 100).unwrap();

    gateway
        .publish(Some(100), EventKind::Trade, 1000, 42)
        .unwrap();

    gateway.expire_sessions(250).unwrap();

    assert_eq!(gateway.active_session_count(), 0);
    assert_eq!(gateway.publisher_stats().pending_deliveries(), 0);

    assert_eq!(gateway.stats().subscriptions_removed, 1);

    gateway.disconnect(id).unwrap();

    // Subscription removal must not be counted twice.
    assert_eq!(gateway.stats().subscriptions_removed, 1);
}

#[test]
fn disconnected_identifier_can_be_reused() {
    let mut gateway = gateway();
    let id = SessionId::new(1);

    gateway.connect(id, 100).unwrap();
    subscribe(&mut gateway, id, 100);

    gateway.disconnect(id).unwrap();

    gateway.connect(id, 200).unwrap();
    subscribe(&mut gateway, id, 200);

    gateway.activate_snapshot(id, 0, 200).unwrap();

    assert_eq!(gateway.session_count(), 1);
    assert_eq!(gateway.active_session_count(), 1);
}
