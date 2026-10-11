//! Calendar domain logic: recurrence, time zones and invitations.
//!
//! Two things here decide whether a meeting shows at the right time. The
//! first is knowing which time zone Microsoft 365 means by the names it
//! uses, which are Windows' and not the ones the rest of the world keeps
//! (`zones`). The second is working out when a repeating event falls: in
//! the event's own zone, so that a weekly nine o'clock meeting stays at nine
//! when the clocks change (`recurrence`).

mod recurrence;
mod zones;

pub use recurrence::{
    DayOfWeek, Ends, Occurrence, Pattern, RecurrenceError, Series, WeekIndex, occurrences,
};
pub use zones::{iana_zone, utc};
