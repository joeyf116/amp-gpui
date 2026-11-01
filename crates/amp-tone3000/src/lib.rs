//! A client for the TONE3000 API.
//!
//! Sign-in is OAuth 2.0 with PKCE in the system browser, and the only credential
//! this crate knows about is the publishable key. TONE3000's secret key is for
//! servers and must never be shipped in the app. Every call blocks, so run them
//! away from the UI and audio threads.
//!
//! The free API tier covers the Select flow and the bounded lists used here, so
//! tone search and a catalog of our own are left out on purpose.

mod client;
mod sign_in;
mod store;
mod types;

pub use client::{Client, Error, List};
pub use sign_in::{Filter, Picked, SignIn};
pub use types::{Creator, Label, Model, Page, Tone, User};

/// The site every link and API call goes to.
pub const WEBSITE: &str = "https://www.tone3000.com";
