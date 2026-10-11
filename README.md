<p align="center"><img src="docs/logo-188.png" width="94" alt="Polystation logo"></p>

# Polystation

A small PlayStation-1 style 3D engine and editor for Windows, written from scratch in Rust with **no
dependencies**: its own OpenGL 3.3 renderer (affine texture mapping, vertex lighting, dithered fog), its own
immediate-mode user interface, a scripting language (GScript) with a visual block editor, a game GUI system,
and a one-click "Compile" that turns a project into a stand-alone `.exe`.

Website: <https://greenru.github.io/polystation/>

## Install

Download **`polystation-setup.exe`** from the [latest release](https://github.com/GreenRu/polystation/releases/latest)
and run it. It downloads the newest `polystation.exe` from this repository's releases, installs it for the
current user, adds a Start menu entry and registers `.polystation` project files. Run the installer again (or
use *Help > Check for Updates*) to update. `polystation-setup.exe --uninstall` removes it.

## Use

* A start page opens with recent projects and **templates** (Blank Room, Wetlands Walk, Coin Collector, Torchlit Hall).
* Right-click objects, drag in the hierarchy to make parents and children, script objects with the `{ }` button
  (code, or a visual graph with wires and an eyedropper), design a game GUI, then press **Compile**.
* An AI assistant can drive the editor through MCP: see [MCP.md](MCP.md).

## Build from source

```
cargo build --release
```

produces `target/release/polystation.exe` (the engine) and `target/release/polystation-setup.exe` (the installer).
Windows only; needs a Rust toolchain.

## License

[MIT](LICENSE): free to use, modify and distribute, as long as the original copyright and license notice are kept with the code.
