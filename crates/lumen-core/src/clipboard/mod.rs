//! Clipboard history and the privacy layer around it (F3).
//!
//! Two halves that are deliberately separable: [`privacy`] decides what may be
//! recorded, and [`store`] records it. Keeping the decision out of the database
//! is what makes the decision testable without one -- and the privacy rules are
//! the part that has to be right.

pub mod images;
pub mod kind;
pub mod privacy;
pub mod session;
pub mod settings;
pub mod store;

pub use kind::{Filter, Kind, kind};
pub use privacy::{Rejected, may_record, may_record_image};
pub use store::{Entry, History, ImageInfo, MAX_ENTRIES_LIMIT, Payload, Retention, Summary};
