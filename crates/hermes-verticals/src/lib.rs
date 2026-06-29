//! Vertical definitions and persona loading for Terra.

pub mod loader;
pub mod persona;

pub use loader::{VerticalDefinition, VerticalLoadError, VerticalLoader};
pub use persona::{PersonaBlock, PersonaBlockKind};
