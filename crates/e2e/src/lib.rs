//! End-to-end test harness for Cutaway.
//!
//! Cucumber scenarios drive the application through the
//! [`driver::ApplicationDriver`] port, so the feature files stay independent
//! of the surface they run against. The in-process driver exercises the
//! application cores (inspection, lenses, planning) directly; a GUI-level
//! driver can implement the same trait later without touching a single
//! feature file.

pub mod driver;
pub mod fakes;
