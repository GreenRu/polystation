# Polystation MCP

Polystation (the PS1-style engine, formerly "gpurender") ships an MCP server so an AI assistant can build
and test scenes: add objects, move them, write GScript, change fog and sky, look at the result, play.

## Connect
Add this to the assistant's MCP config (a ready one is in `.mcp.json` at the repo root):

    { "mcpServers": { "polystation": { "command": "<path>\polystation.exe", "args": ["--mcp"] } } }

`polystation --mcp` speaks MCP over stdin/stdout. It starts the editor itself if it is not running
(set `POLYSTATION_PROJECT` to open a project then). The editor listens on `127.0.0.1:47800`.

## Tools
get_info, get_scene, get_hierarchy, compile, get_gui, set_gui_mode, add_gui, modify_gui, delete_gui, set_parent, add_object, modify_object, delete_object, select_object, set_script, get_script,
get_guide, set_environment, set_camera, screenshot (PNG), play, history (undo/redo), open_project,
import_model, save_project.

Edits go through the same undo history as the UI. Edits are refused while the game is playing.
Suggested flow: get_info -> get_scene -> add_object / set_script (read get_guide first) -> screenshot.
