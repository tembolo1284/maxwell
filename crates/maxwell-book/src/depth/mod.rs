//! Read-only market depth representations.
//!
//! L1: Best bid and offer.
//! L2: Aggregated price levels.
//! L3: Individual resting orders.
//!
//! All representations derive from the same underlying
//! authoritative order book state.

pub mod level1;
pub mod level2;
pub mod level3;

pub use level1::{Level1, Quote};

pub use level2::{DepthLevel, Level2};

pub use level3::Level3;
