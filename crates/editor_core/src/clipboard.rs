//! The editor's in-app clipboards (design §6.3): one value per type, so a kit or a domain keeps what its kind of tab
//! copied (`curve_kit`: points) next to the documents, inside `Editor`. Every tab of every domain sees the same
//! store through [`crate::doc_kind::DocCtx::clipboards`]; text for other applications is
//! [`crate::state::editor::Editor::copy_text`].

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// What the tabs copied, keyed by the type of the copied value.
#[derive(Default)]
pub struct Clipboards(HashMap<TypeId, Box<dyn Any + Send + Sync>>);

impl Clipboards {
    /// The copied `T`, if any.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.0.get(&TypeId::of::<T>()).and_then(|v| v.downcast_ref())
    }

    pub fn get_mut<T: Any + Send + Sync>(&mut self) -> Option<&mut T> {
        self.0.get_mut(&TypeId::of::<T>()).and_then(|v| v.downcast_mut())
    }

    /// Replaces the copied `T`.
    pub fn set<T: Any + Send + Sync>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Box::new(value));
    }
}

impl std::fmt::Debug for Clipboards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clipboards").field("types", &self.0.len()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_value_per_type() {
        let mut c = Clipboards::default();
        assert!(c.get::<u32>().is_none());
        c.set(7u32);
        c.set(String::from("a"));
        *c.get_mut::<u32>().expect("u32") += 1;
        c.set(String::from("b"));
        assert_eq!((c.get::<u32>(), c.get::<String>().map(String::as_str)), (Some(&8), Some("b")));
        assert!(c.get::<i32>().is_none());
    }
}
