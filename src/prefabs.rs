//! Ready-made gameplay objects of the Add > Gameplay menu: a shape plus its size, color, collision
//! mode and a script, all of which can be changed afterwards like on any other object.
use crate::light::{LightKind, LightParams};
use crate::prims::Prim;
use crate::scene::Collision;

pub struct Prefab {
    pub name: &'static str,
    pub prim: Prim,
    /// Size in world units (the player is 1.8 tall).
    pub scale: [f32; 3],
    pub color: [f32; 3],
    pub alpha: f32,
    pub collision: Collision,
    pub script: &'static str,
    /// Settings when the prefab is a light.
    pub light: Option<LightParams>,
}

pub const PREFABS: [Prefab; 8] = [
    Prefab {
        name: "Trigger Zone",
        prim: Prim::Cube,
        scale: [4.0, 3.0, 4.0],
        color: [0.3, 0.55, 1.0],
        alpha: 0.25,
        collision: Collision::Trigger,
        light: None,
        script: "// An area that notices the player. Change what happens below.\non touch {\n    print(\"entered \" + self.name)\n}\n\non leave {\n    print(\"left \" + self.name)\n}\n",
    },
    Prefab {
        name: "Coin",
        prim: Prim::Disc,
        scale: [0.8, 0.8, 0.8],
        color: [1.0, 0.8, 0.1],
        alpha: 1.0,
        collision: Collision::Trigger,
        light: None,
        script: "// Spins, and is collected when the player touches it.\n// The count is shared: get(\"coins\") works from any object's script.\non update {\n    spin(0, 180, 0)\n}\n\non touch {\n    add(\"coins\", 1)\n    hud(\"Coins: \" + get(\"coins\"))\n    destroy()\n}\n",
    },
    Prefab {
        name: "Sliding Wall",
        prim: Prim::Cube,
        scale: [4.0, 3.0, 0.5],
        color: [0.6, 0.6, 0.65],
        alpha: 1.0,
        collision: Collision::Solid,
        light: None,
        script: "// Slides back and forth along x. Change the numbers to taste.\nvar start_x = 0\nvar reach = 5\nvar speed = 1\n\non start {\n    start_x = position.x\n}\n\non update {\n    position.x = start_x + sin(time * speed) * reach\n}\n",
    },
    Prefab {
        name: "Spinner",
        prim: Prim::Cube,
        scale: [6.0, 0.5, 0.5],
        color: [0.9, 0.3, 0.3],
        alpha: 1.0,
        collision: Collision::Solid,
        light: None,
        script: "// A bar that turns around its middle.\non update {\n    spin(0, 90, 0)\n}\n",
    },
    Prefab {
        name: "Door",
        prim: Prim::Cube,
        scale: [2.0, 3.0, 0.3],
        color: [0.55, 0.38, 0.2],
        alpha: 1.0,
        collision: Collision::Solid,
        light: None,
        script: "// Press E near the door to open / close it by sliding it up.\nvar closed_y = 0\nvar open = false\n\non start {\n    closed_y = position.y\n}\n\non update {\n    if pressed(\"e\") and distance(self, player) < 3 {\n        open = !open\n    }\n    var target = closed_y\n    if open {\n        target = closed_y + 3\n    }\n    position.y = lerp(position.y, target, 5 * dt)\n}\n",
    },
    Prefab {
        name: "Jump Pad",
        prim: Prim::Disc,
        scale: [2.0, 1.0, 2.0],
        color: [0.9, 0.2, 0.8],
        alpha: 1.0,
        collision: Collision::Trigger,
        light: None,
        script: "// Throws the player up. launch(12) = a jump 12 units high.\non touch {\n    launch(12)\n}\n",
    },
    Prefab {
        name: "Water",
        prim: Prim::Cube,
        scale: [20.0, 4.0, 20.0],
        color: [0.15, 0.4, 0.8],
        alpha: 0.45,
        collision: Collision::Water,
        light: None,
        script: "",
    },
    Prefab {
        name: "Torch",
        prim: Prim::Light,
        scale: [1.0, 1.0, 1.0],
        color: [1.0, 0.62, 0.28],
        alpha: 1.0,
        collision: Collision::Passable,
        light: Some(LightParams { kind: LightKind::Point, intensity: 1.4, range: 14.0, angle: 50.0 }),
        script: "// A flickering warm light.\non update {\n    intensity = 1.3 + sin(time * 13) * 0.12 + sin(time * 7.3) * 0.1\n}\n",
    },
];
