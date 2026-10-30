//! Canvas gestures, Material's half: the held-key node placement data (decision 0032). The gestures themselves and
//! the minimap are `node_graph_base`'s ([`InputPlugin`] adds them for Material's graph).

use bevy::prelude::*;

use crate::graph_domain::MaterialGraph;

/// Material's canvas gestures (`node_graph_base`'s for [`MaterialGraph`]; the window app adds them through
/// `register_graph`, UI tests add this).
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(node_graph_base::input::InputPlugin::<MaterialGraph>::default());
    }
}

/// Where the cursor is over Material's graph canvas.
pub type CanvasCursor<'w, 's> = node_graph_base::input::CanvasCursor<'w, 's, MaterialGraph>;

/// UE: hold a key and click empty canvas to drop a node there (decision 0032).
pub(crate) const NODE_HOTKEYS: [(KeyCode, &str); 18] = {
    use KeyCode::*;
    [
        (Digit1, "stdlib::Constant"),
        (Digit2, "stdlib::Constant2Vector"),
        (Digit3, "stdlib::Constant3Vector"),
        (Digit4, "stdlib::Constant4Vector"),
        (KeyS, "param::Scalar"),
        (KeyV, "param::Vector"),
        (KeyT, "stdlib::TextureSample"),
        (KeyM, "stdlib::Multiply"),
        (KeyA, "stdlib::Add"),
        (KeyL, "stdlib::Lerp"),
        (KeyU, "stdlib::TextureCoordinate"),
        (KeyR, "stdlib::Reroute"),
        (KeyD, "stdlib::Divide"),
        (KeyO, "stdlib::OneMinus"),
        (KeyE, "stdlib::Power"),
        (KeyN, "stdlib::Normalize"),
        (KeyP, "stdlib::Panner"),
        (KeyI, "stdlib::If"),
    ]
};

#[cfg(test)]
mod tests {
    use super::*;
    use node_graph_base::input::hotkey_node;

    #[test]
    fn hotkey_nodes_exist() {
        let registry = shader_stdlib::registry();
        for (key, kind) in NODE_HOTKEYS {
            assert!(registry.get(kind).is_some(), "{key:?} drops `{kind}`, which is not registered");
        }
        // UE keys we have nodes for (no BumpOffset in the stdlib).
        let ue = [
            (KeyCode::KeyD, "stdlib::Divide"),
            (KeyCode::KeyO, "stdlib::OneMinus"),
            (KeyCode::KeyE, "stdlib::Power"),
            (KeyCode::KeyN, "stdlib::Normalize"),
            (KeyCode::KeyP, "stdlib::Panner"),
            (KeyCode::KeyI, "stdlib::If"),
        ];
        for (key, kind) in ue {
            let mut keys = ButtonInput::<KeyCode>::default();
            keys.press(key);
            assert_eq!(hotkey_node(&keys, &NODE_HOTKEYS), Some(kind), "{key:?}");
        }
        let keys: Vec<KeyCode> = NODE_HOTKEYS.iter().map(|(k, _)| *k).collect();
        assert!(keys.iter().all(|k| keys.iter().filter(|o| *o == k).count() == 1), "one node per key");
        // Plain keys only (decision 0032): Ctrl+D, Alt+N or Super+A held place nothing; Shift is allowed.
        for m in [KeyCode::ControlLeft, KeyCode::AltRight, KeyCode::SuperLeft] {
            for key in [KeyCode::KeyD, KeyCode::KeyN, KeyCode::KeyA] {
                let mut keys = ButtonInput::<KeyCode>::default();
                keys.press(m);
                keys.press(key);
                assert_eq!(hotkey_node(&keys, &NODE_HOTKEYS), None, "{m:?}+{key:?}");
            }
        }
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::KeyD);
        assert_eq!(hotkey_node(&keys, &NODE_HOTKEYS), Some("stdlib::Divide"));
    }
}
