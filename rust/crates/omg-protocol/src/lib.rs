//! OpenMyGlass companion-link wire protocol, **version 1**.
//!
//! This crate is the *contract* between the two peers of the OpenMyGlass link:
//!
//! * the **companion** (`omg-*` crates in this workspace), running on a phone or PC, and
//! * **glassd**, the Rust daemon on Google Glass (separate repo/agent).
//!
//! Both ends depend on this crate so the schema cannot drift: the types here are
//! the single source of truth. glassd must implement the same [`Frame`] enum and the
//! same framing (see [`wire`]).
//!
//! ## Design
//!
//! * Pure data + serde. No async runtime, no transport, no I/O — so the exact same
//!   crate compiles into the companion (std) and into glassd.
//! * One [`Frame`] enum carries every message. Backbone variants (session, pairing,
//!   auth, locale, status/battery, notifications) are fully specified and implemented
//!   end-to-end in `omg-core`. The remaining variants are typed and documented but
//!   their handlers are scaffolded (see the `// SCAFFOLD` markers).
//! * Wire format: length-delimited [`postcard`] (compact, `no_std`-friendly, ideal for
//!   the Glass). A JSON projection is available for debugging/tests via serde.
//!
//! ## Versioning
//!
//! [`PROTOCOL_VERSION`] is sent in [`messages::Hello`]. Peers negotiate the minimum of
//! the two. Because postcard enums are encoded by discriminant, **new variants must be
//! appended at the end of [`Frame`]** and a peer must tolerate (log + skip) a
//! [`WireError::UnknownVariant`] from a newer peer for anything beyond its known set.
//!
//! Today's decoders do not do that yet: an unknown discriminant surfaces as a postcard
//! error and closes the link (on glassd too). So a feature group that adds variants has
//! to be gated by a [`capability`], as [`notes`] is.

pub mod bootstrap;
pub mod error;
pub mod locale;
pub mod messages;
pub mod notes;
pub mod wire;

pub use error::WireError;
pub use locale::Locale;
pub use messages::*;
pub use notes::*;
pub use wire::{decode_payload, encode_payload, frame_bytes, FrameDecoder, MAX_FRAME_LEN};

/// Protocol version advertised in [`messages::Hello`]. Bump on any breaking schema change.
pub const PROTOCOL_VERSION: u16 = 1;

/// Capability strings advertised in [`messages::Hello::capabilities`].
///
/// Kept as plain strings (not an enum) so a peer can advertise capabilities a
/// counterpart does not know about yet without breaking decode.
pub mod capability {
    pub const NOTIFICATIONS: &str = "notifications";
    pub const DICTATION: &str = "dictation";
    pub const MEDIA_SYNC: &str = "media-sync";
    pub const LOCATION: &str = "location";
    pub const NAVIGATION: &str = "navigation";
    pub const CAPTIONS: &str = "captions";
    pub const ASSISTANT: &str = "assistant";
    pub const ADDONS: &str = "addons";
    pub const SCREENCAST: &str = "screencast";
    pub const OTA: &str = "ota";
    pub const TETHERING: &str = "tethering";
    pub const RADIO_CONTROL: &str = "radio-control";
    pub const AIRPLANE: &str = "airplane";
    pub const LOCALE: &str = "locale";
    /// Notes with attachments and deferred sync ([`crate::notes`]). Deliberately not in
    /// [`BACKBONE`]: a peer advertises it only once it implements those frames, and no
    /// note frame may be sent to a peer that did not advertise it.
    pub const NOTES_SYNC: &str = "notes-sync";

    /// Everything the current companion core knows how to speak at the backbone level.
    pub const BACKBONE: &[&str] = &[NOTIFICATIONS, LOCALE, RADIO_CONTROL, AIRPLANE];
}
