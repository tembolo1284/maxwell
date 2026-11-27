
//! Multi-instrument order book coordinator.
//!
//! Each instrument owns an independent order book.
//! Each book is bound to one authoritative feed session.

use std::collections::HashMap;

use maxwell_core::{
    FeedId,
    Instrument,
    InstrumentId,
};

use crate::book::OrderBook;
use crate::config::BookConfig;
use crate::error::{BookError, BookResult};
use crate::snapshot::BookSnapshot;
use crate::update::BookUpdate;

#[derive(Debug)]
pub struct BookManager {
    config: BookConfig,
    books: HashMap<InstrumentId, OrderBook>,
}

impl BookManager {
    pub fn new(config: BookConfig) -> BookResult<Self> {
        config.validate()?;

        let mut books = HashMap::new();

        books
            .try_reserve(config.max_instruments)
            .map_err(|_| BookError::AllocationFailed)?;

        Ok(Self {
            config,
            books,
        })
    }

    #[must_use]
    #[inline]
    pub const fn config(&self) -> BookConfig {
        self.config
    }

    #[must_use]
    #[inline]
    pub fn instrument_count(&self) -> usize {
        self.books.len()
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }

    /// Registers an instrument with an initially empty book.
    ///
    /// Registration belongs to the control path.
    pub fn register(
        &mut self,
        instrument: Instrument,
        feed: FeedId,
    ) -> BookResult<()> {
        let id = instrument.id();

        if self.books.contains_key(&id) {
            return Err(BookError::InstrumentAlreadyRegistered(id));
        }

        if self.books.len() >= self.config.max_instruments {
            return Err(BookError::InstrumentCapacityExceeded);
        }

        let book = OrderBook::new(
            instrument,
            feed,
            self.config,
        )?;

        self.books.insert(id, book);

        Ok(())
    }

    #[must_use]
    #[inline]
    pub fn book(&self, id: InstrumentId) -> Option<&OrderBook> {
        self.books.get(&id)
    }

    pub fn remove(&mut self, id: InstrumentId) -> Option<OrderBook> {
        self.books.remove(&id)
    }

    /// Routes a sequenced update to its instrument.
    #[inline]
    pub fn apply(&mut self, update: BookUpdate) -> BookResult<()> {
        let book = self.books
            .get_mut(&update.instrument_id)
            .ok_or(BookError::UnknownInstrument(update.instrument_id))?;

        book.apply(update)
    }

    pub fn require_recovery(
        &mut self,
        id: InstrumentId,
    ) -> BookResult<()> {
        let book = self.books
            .get_mut(&id)
            .ok_or(BookError::UnknownInstrument(id))?;

        book.require_recovery();

        Ok(())
    }

    /// Invalidates every instrument associated with a feed.
    ///
    /// A missing stream event could affect any instrument
    /// carried by that stream.
    pub fn require_feed_recovery(&mut self, feed: FeedId) {
        for book in self.books.values_mut() {
            if book.feed() == feed {
                book.require_recovery();
            }
        }
    }

    pub fn capture_snapshot(
        &self,
        id: InstrumentId,
    ) -> BookResult<BookSnapshot> {
        let book = self.books
            .get(&id)
            .ok_or(BookError::UnknownInstrument(id))?;

        BookSnapshot::capture(book)
    }

    pub fn install_snapshot(
        &mut self,
        snapshot: &BookSnapshot,
    ) -> BookResult<()> {
        let id = snapshot.instrument.id();

        let book = self.books
            .get_mut(&id)
            .ok_or(BookError::UnknownInstrument(id))?;

        snapshot.install(book)
    }

    /// Installs an authoritative snapshot for a new session.
    ///
    /// The caller must authenticate the new session and
    /// coordinate the transition with the sequencer.
    pub fn install_new_session(
        &mut self,
        snapshot: &BookSnapshot,
    ) -> BookResult<()> {
        let id = snapshot.instrument.id();

        let current = self.books
            .get_mut(&id)
            .ok_or(BookError::UnknownInstrument(id))?;

        if snapshot.instrument != *current.instrument()
            || snapshot.feed == current.feed()
        {
            return Err(BookError::InvalidSnapshot);
        }

        let restored = snapshot.restore(self.config)?;

        *current = restored;

        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &OrderBook> {
        self.books.values()
    }
}

