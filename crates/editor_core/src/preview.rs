//! Domain-agnostic RAII preview session lifecycle (`start_preview`, `clean_preview`, [`PreviewHandle`]).
//!
//! Any open document ([`crate::doc_kind::TabDoc::preview_handle`]) or standalone system ([`PreviewTracker::track`])
//! may hold a [`PreviewHandle`]. [`sync_previews`] reconciles live handles against [`PreviewTracker`] every frame,
//! invoking the domain's registered [`PreviewCallbacks::start`] for newly opened previews and
//! [`PreviewCallbacks::clean`] (plus automatic despawning of all [`PreviewEntity`] entities) when a handle is
//! explicitly cleaned ([`clean_preview`] / [`PreviewHandle::clean`]) or dropped.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bevy::prelude::*;

use crate::state::editor::Editor;

static NEXT_PREVIEW_ID: AtomicU64 = AtomicU64::new(1);

/// Namespaced identifier of a domain's preview kind (`"light.preview_3d"`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PreviewKindId(pub &'static str);

impl fmt::Display for PreviewKindId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Tags any ECS entity (camera, directional light, demo scene mesh, etc.) spawned for preview handle `0`.
///
/// [`sync_previews`] automatically despawns all entities with `PreviewEntity(id)` when preview `id` is cleaned up.
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PreviewEntity(pub u64);

/// Start and cleanup hooks registered by a domain for a [`PreviewKindId`].
#[derive(Clone, Copy, Debug)]
pub struct PreviewCallbacks {
    pub kind: PreviewKindId,
    pub start: fn(&mut World, u64),
    pub clean: fn(&mut World, u64),
}

/// RAII handle for an active preview session (`u64` id).
///
/// Created by [`start_preview`]. When dropped (or when [`clean_preview`] / [`PreviewHandle::clean`] is called),
/// the handle marks its session for cleanup so [`sync_previews`] despawns all [`PreviewEntity`] entities,
/// cameras, lights, demo scene meshes, and resources associated with `id`.
#[derive(Debug)]
pub struct PreviewHandle {
    id: u64,
    kind: PreviewKindId,
    alive: Arc<AtomicBool>,
}

impl PreviewHandle {
    /// Unique non-zero session identifier.
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Preview kind that owns this session.
    pub const fn kind(&self) -> PreviewKindId {
        self.kind
    }

    /// Whether this preview handle is still active (`false` once [`PreviewHandle::clean`] or [`clean_preview`] has been
    /// called).
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Marks this preview handle for immediate cleanup on the next [`sync_previews`] pass without requiring the owner
    /// to drop the struct first.
    pub fn clean(&self) {
        self.alive.store(false, Ordering::Release);
    }
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        self.clean();
    }
}

/// Allocates a new live [`PreviewHandle`] with a unique non-zero `u64` id for `kind`.
pub fn start_preview(kind: PreviewKindId) -> PreviewHandle {
    let id = NEXT_PREVIEW_ID.fetch_add(1, Ordering::Relaxed);
    PreviewHandle { id, kind, alive: Arc::new(AtomicBool::new(true)) }
}

/// Explicitly marks `handle` for cleanup and drops it so [`sync_previews`] tears down all associated preview entities
/// and resources.
pub fn clean_preview(handle: PreviewHandle) {
    handle.clean();
    drop(handle);
}

/// Tracks active preview sessions and dispatches domain [`PreviewCallbacks`] as [`PreviewHandle`]s are created,
/// cleaned, or dropped.
#[derive(Resource, Default)]
pub struct PreviewTracker {
    callbacks: BTreeMap<PreviewKindId, PreviewCallbacks>,
    spawned: BTreeMap<u64, PreviewKindId>,
    extra: BTreeMap<u64, (PreviewKindId, Arc<AtomicBool>)>,
}

impl PreviewTracker {
    /// Builds a tracker with the domains' registered [`PreviewCallbacks`].
    pub fn new(callbacks: impl IntoIterator<Item = PreviewCallbacks>) -> Self {
        let mut tracker = Self::default();
        for cb in callbacks {
            tracker.register(cb);
        }
        tracker
    }

    /// Registers callbacks for `callbacks.kind`.
    pub fn register(&mut self, callbacks: PreviewCallbacks) {
        self.callbacks.insert(callbacks.kind, callbacks);
    }

    /// Tracks a standalone [`PreviewHandle`] not owned by a [`crate::doc_kind::TabDoc`].
    pub fn track(&mut self, handle: &PreviewHandle) {
        if handle.is_alive() {
            self.extra.insert(handle.id, (handle.kind, Arc::clone(&handle.alive)));
        }
    }

    /// Whether preview session `id` is currently spawned in the ECS world.
    pub fn is_spawned(&self, id: u64) -> bool {
        self.spawned.contains_key(&id)
    }

    /// All currently spawned `(id, kind)` preview sessions, in deterministic ID order.
    pub fn spawned_ids(&self) -> impl Iterator<Item = (u64, PreviewKindId)> + '_ {
        self.spawned.iter().map(|(&id, &k)| (id, k))
    }
}

/// Reconciles live [`PreviewHandle`]s (from open [`Editor`] tabs and [`PreviewTracker::track`]) against spawned
/// preview sessions, running `clean` (and despawning [`PreviewEntity`] entities) for ended sessions and `start` for
/// newly created ones.
pub fn sync_previews(world: &mut World) {
    if !world.contains_resource::<PreviewTracker>() {
        world.init_resource::<PreviewTracker>();
    }

    let mut live: BTreeMap<u64, PreviewKindId> = BTreeMap::new();
    if let Some(editor) = world.get_resource::<Editor>() {
        for tab in &editor.tabs {
            if let Some(h) = tab.doc().preview_handle()
                && h.is_alive()
            {
                live.insert(h.id(), h.kind());
            }
        }
    }

    let (to_clean, to_start) = {
        let mut tracker = world.resource_mut::<PreviewTracker>();
        tracker.extra.retain(|_, (_, alive)| alive.load(Ordering::Acquire));
        for (&id, &(kind, _)) in &tracker.extra {
            live.insert(id, kind);
        }

        let to_clean: Vec<(u64, Option<PreviewCallbacks>)> = tracker
            .spawned
            .iter()
            .filter(|(id, _)| !live.contains_key(id))
            .map(|(&id, kind)| (id, tracker.callbacks.get(kind).copied()))
            .collect();

        let to_start: Vec<(u64, PreviewKindId, Option<PreviewCallbacks>)> = live
            .iter()
            .filter(|(id, _)| !tracker.spawned.contains_key(id))
            .map(|(&id, &kind)| (id, kind, tracker.callbacks.get(&kind).copied()))
            .collect();

        for (id, _) in &to_clean {
            tracker.spawned.remove(id);
        }
        for &(id, kind, _) in &to_start {
            tracker.spawned.insert(id, kind);
        }

        (to_clean, to_start)
    };

    for (id, cb) in to_clean {
        if let Some(cb) = cb {
            (cb.clean)(world, id);
        }
        let entities: Vec<Entity> = world
            .query::<(Entity, &PreviewEntity)>()
            .iter(world)
            .filter_map(|(e, pe)| (pe.0 == id).then_some(e))
            .collect();
        for entity in entities {
            if world.get_entity(entity).is_ok() {
                world.despawn(entity);
            }
        }
    }

    for (id, _kind, cb) in to_start {
        if let Some(cb) = cb {
            (cb.start)(world, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KIND: PreviewKindId = PreviewKindId("test.preview");

    #[derive(Resource, Default)]
    struct LifecycleLog {
        started: Vec<u64>,
        cleaned: Vec<u64>,
    }

    fn on_start(world: &mut World, id: u64) {
        world.resource_mut::<LifecycleLog>().started.push(id);
        world.spawn((PreviewEntity(id), Name::new(format!("cam_{id}"))));
        world.spawn((PreviewEntity(id), Name::new(format!("mesh_{id}"))));
    }

    fn on_clean(world: &mut World, id: u64) {
        world.resource_mut::<LifecycleLog>().cleaned.push(id);
    }

    #[test]
    fn start_preview_allocates_unique_ids_and_cleans_on_drop_or_explicit_clean() {
        let mut world = World::new();
        world.init_resource::<LifecycleLog>();
        let mut tracker = PreviewTracker::default();
        tracker.register(PreviewCallbacks { kind: TEST_KIND, start: on_start, clean: on_clean });

        let h1 = start_preview(TEST_KIND);
        let h2 = start_preview(TEST_KIND);
        assert!(h1.id() > 0 && h2.id() > h1.id());
        assert!(h1.is_alive() && h2.is_alive());

        tracker.track(&h1);
        tracker.track(&h2);
        world.insert_resource(tracker);

        sync_previews(&mut world);
        assert!(world.resource::<PreviewTracker>().is_spawned(h1.id()));
        assert!(world.resource::<PreviewTracker>().is_spawned(h2.id()));
        assert_eq!(world.resource::<LifecycleLog>().started, vec![h1.id(), h2.id()]);
        assert_eq!(world.query::<&PreviewEntity>().iter(&world).count(), 4);

        // Explicit clean_preview on h1 tears down only h1's entities and runs on_clean(h1).
        let id1 = h1.id();
        let id2 = h2.id();
        clean_preview(h1);
        sync_previews(&mut world);
        assert!(!world.resource::<PreviewTracker>().is_spawned(id1));
        assert!(world.resource::<PreviewTracker>().is_spawned(id2));
        assert_eq!(world.resource::<LifecycleLog>().cleaned, vec![id1]);
        let remaining: Vec<u64> = world.query::<&PreviewEntity>().iter(&world).map(|e| e.0).collect();
        assert_eq!(remaining, vec![id2, id2]);

        // Dropping h2 (RAII) cleans up h2 on the next sync_previews pass.
        drop(h2);
        sync_previews(&mut world);
        assert!(!world.resource::<PreviewTracker>().is_spawned(id2));
        assert_eq!(world.resource::<LifecycleLog>().cleaned, vec![id1, id2]);
        assert_eq!(world.query::<&PreviewEntity>().iter(&world).count(), 0);
    }
}
