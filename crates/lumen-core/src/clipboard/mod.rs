//! Clipboard history and the privacy layer around it (F3).
//!
//! Two halves that are deliberately separable: [`privacy`] decides what may be
//! recorded, and [`store`] records it. Keeping the decision out of the database
//! is what makes the decision testable without one -- and the privacy rules are
//! the part that has to be right.

pub mod kind;
pub mod privacy;
pub mod session;
pub mod store;

pub use kind::{Filter, Kind, kind};
pub use privacy::{Rejected, may_record};
pub use store::{Entry, History, Payload, Retention};
