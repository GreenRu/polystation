//! The scripting guide shown in the GUIDE tab, and the example scripts of the Help menu.

pub const GUIDE: &str = r#"GSCRIPT GUIDE
=============
A script is a small piece of code that belongs to ONE object. When you press
Play it can move, turn, recolor or hide that object, and react to the player.

HOW TO ADD ONE
  1. Click an object in the Hierarchy (or in the 3D view).
  2. Right-click it > Add Script  (or open the SCRIPT tab and start typing).
  3. Write code. The line above the editor says "script OK", or shows the line
     that is wrong.
  4. Press Play (F5). Esc stops - everything goes back the way it was.
  Help > Example Scripts inserts ready-made scripts that you can change.

THE SHAPE OF A SCRIPT
  A script is a list of events. An event is a block of code that runs when
  something happens:

    on start  { ... }        once, when Play begins
    on update { ... }        every frame (60+ times a second)
    on key("e") { ... }      once each time that key goes down
    on every(2) { ... }      again and again, every 2 seconds
    on touch  { ... }        when the player walks into this object
    on leave  { ... }        when the player walks out of it again
                             (touch / leave need Collision = Trigger)

  Outside the events you may only write "var" (a memory) and "fn" (your own
  function).

REMEMBERING THINGS: var
    var speed = 90                    // outside events: kept the whole game
    on update { rotation.y += speed * dt }

  Inside an event, "var" makes a short-lived variable that disappears at the
  closing brace.

TIME: dt AND time
    dt    seconds since the last frame (about 0.016)
    time  seconds since you pressed Play
  Multiply movement by dt so it is the same speed on every computer:
    position.x += 5 * dt              // 5 units per second

THE OBJECT'S PROPERTIES
  (written bare, they belong to the object that owns the script)
    position.x  position.y  position.z     where it is (y is up)
    rotation.x  rotation.y  rotation.z     turn, in degrees
    scale.x     scale.y     scale.z        size (1 = normal)
    color.r     color.g     color.b        0 to 1
    alpha                                  0 = invisible ... 1 = solid
    visible                                true / false
    solid                                  true / false: does the player bump into it?
  Use them like variables:
    position.y = 3        position.y += 1        if alpha < 0.5 { ... }

OTHER OBJECTS AND THE PLAYER
    object("Door").position.y = 3          the object named Door
    object("Door").visible = false
    player.x  player.y  player.z           where the player is (set them to teleport)
    player.rotation.y                      which way the player looks
    self                                   the object that owns the script

MAKING DECISIONS
    if distance(self, player) < 3 { ... } else { ... }
  Compare with  ==  !=  <  <=  >  >=     Combine with  and  or  not
  Text goes in quotes and joins with +:   print("score " + score)

REPEATING
    for i in 0..5 { ... }       i is 0, 1, 2, 3, 4
    while condition { ... }     (a loop that never ends is stopped for you)
    break      continue

YOUR OWN FUNCTIONS
    fn bounce(height) { return sin(time * 3) * height }
    on update { position.y = 1 + bounce(0.5) }

BUILT-IN FUNCTIONS
    move(x, y, z)       slide this object x, y, z units per second
    spin(x, y, z)       turn this object x, y, z degrees per second
                        (use both inside  on update )
    key("w")            true while the key is held
    pressed("e")        true only on the frame the key goes down
    distance(a, b)      how far apart two objects are: distance(self, player)
    object("Name")      find an object by name      exists("Name")  is there one?
    parent(x)           the object x is inside (nil if none)       children(x)  how many objects are inside x
    child(x, n)         the n-th object inside x (1 = first, nil if none)
    set_parent(x, p)    put x inside p (set_parent(x, nil) = top level)
                        Parent / child objects are arranged by dragging in the explorer. Moving,
                        turning or scaling a parent (also from a script) carries its children along.
    GUI                 elements made in View > Game GUI Editor (text, panels, bars, buttons) are
                        shown on screen while playing. Scripts control them by name:
    gui_text("Score", "Coins: 5")   set the text (gui_text("Score") reads it)
    gui_value("Health", 0.5)        fill of a bar, 0 to 1 (gui_value("Health") reads it)
    gui_show("Menu", false)         show / hide      gui_move("Score", 20, 40)   x, y
    gui_color("Health", 1, 0, 0)    red, green, blue 0 to 1
    camera(object("Cam"))           look through a Camera object (Add > Camera; it looks along its -Z, turn
                                    it with Rotation); camera(player) returns to the player's eyes
    gui_mode("cursor")              how the mouse works: "look" (turns the camera), "cursor" (a pointer that
                                    clicks buttons), "toggle" (Tab switches) or "hold" (Ctrl); set in the GUI editor
    cursor(true)                    force the pointer on / off from a script ("auto" = follow the mode)
    clicked("Start")                true on the frame a button was clicked
    MOUSE               the pointer starts in the middle of the game screen when playing starts.
    mouse_lock(true)                keep the pointer inside the game view (false lets it leave again)
    mouse_center()                  move the pointer to the middle of the game view
    mouse_set(x, y)                 move the pointer to a spot of the game view, in pixels
    mouse_cursor("hand")            pointer look: arrow hand cross text wait move no resize_h resize_v,
                                    "none" hides it, "auto" is back to normal
    mouse_x()  mouse_y()  mouse_down()   pointer position (pixels of the game view) and left button
                                    (while the pointer is free, see cursor(true) / gui_mode)
    quit()                          ends the game (stops Play in the editor)
    save_value("coins", 5)          keep a number, text or true / false between runs of the game
    load_value("coins", 0)          read it back (the second value is used when nothing was saved)
    random()            0 to 1                      random(a, b)    between a and b
    sin cos tan atan2 abs sqrt floor ceil round sign pow min max
    clamp(v, lo, hi)    lerp(a, b, t)               (t = 0 gives a, 1 gives b)
    print("text")       shows a message in the status bar
    str(x)              turns a number into text      round(x, 2)  rounds to 2 decimals
    destroy()           hides this object and makes it passable (destroy(object("Door")) too)
    launch(12)          throws the player up so the jump is 12 units high
    set("name", v)      remember a value that ALL scripts can see
    get("name")         read it back (nothing yet = nil)       add("name", 1)  adds to it
    hud("text")         text that stays in the status bar while playing
    sound("coin.wav")   plays a .wav file (a relative path is looked up next to the project)
  Key names: "a" to "z", "0" to "9", "space", "shift", "ctrl", "alt", "enter",
  "tab", "escape", "up", "down", "left", "right", "click", "rightclick".

GAMEPLAY OBJECTS  (Add > Gameplay)
  Ready-made objects that already have a script, a size and a collision mode:
    Trigger Zone    an area that prints when the player enters / leaves
    Coin            spins; collecting it adds 1 to "coins" and shows it in the status bar
    Sliding Wall    slides back and forth
    Spinner         a bar that turns round and round
    Door            press E near it to open / close it
    Jump Pad        throws the player into the air
    Water           a see-through floor you can walk through
  Open the SCRIPT tab to see and change what they do.
  Other shapes: Add > Shapes (cube, sphere, capsule, pyramid, disc ...), Building
  (ramp, stairs, arch) and Nature (terrain, tree, rock).

TREES  (Add > Nature > Tree)
  A tree object you can shape: Kind (pine, oak, birch, palm, dead), how much trunk, trunk width,
  crown width, Detail (layers / leaf clumps / fronds) and a Seed; "New random tree" tries another.
  The colors come from a built-in palette, so no texture is needed. Scale the object for size.
  A Scatter layer on a terrain plants many trees (its Shape list has the same kinds, each planted
  in a few slightly different versions).

LIGHTS  (Add > Light)
    Point    shines in every direction and fades out at its Range
    Spot     a cone of light (Angle = how wide); rotate the object to aim it
    Sun      parallel light from far away; only its direction matters
  A light shines down the object's -Y axis. The object's color tints the light.
  Lighting is worked out at the corners (vertices) of every face and blended across it, like on
  old consoles. A big flat face has only its corners to catch light, so floors and walls look best
  made of Terrain or small pieces (the Cube and Plane shapes are already cut up for this).
  A scene with no lights is lit by one fixed sun as before. Once there is a light, the Environment's
  Ambient light is what shows in the shadows. Scripts can change `intensity`, `color` and `visible`
  of a light, which makes flickering torches easy (see Add > Gameplay > Torch).

COLLISION  (Inspector > Collision, or right-click > Collision)
    Solid      the player cannot walk through it (the default)
    Passable   the player walks straight through it
    Trigger    passable, and runs  on touch / on leave  in its script
    Water      a pool: the player wades, then swims (hold Space to swim up); the
               surface ripples. Make the box deep enough to swim in.
  A script can change it while playing:   solid = false
  Player > Max Step is how tall a ledge the player walks up without jumping.

IF SOMETHING GOES WRONG
  "unknown name x"          you used a variable that was never created: var x = 0
  "put code inside on ..."  write it inside  on start { }  or  on update { }
  Things move too fast      multiply by dt, or use move() / spin()
  object("Dor") fails       the name must match the Hierarchy exactly
  Nothing happens           scripts only run while playing (press Play)
"#;

/// (menu title, script text)
pub const EXAMPLES: [(&str, &str); 11] = [
    (
        "Spin around",
        "// Turns the object 90 degrees every second.\non update {\n    spin(0, 90, 0)\n}\n",
    ),
    (
        "Bob up and down",
        "// Floats up and down around where the object started.\nvar base = 0\n\non start {\n    base = position.y\n}\n\non update {\n    position.y = base + sin(time * 2) * 0.5\n}\n",
    ),
    (
        "Pulse color",
        "// Fades the red channel in and out.\non update {\n    color.r = 0.5 + 0.5 * sin(time * 3)\n}\n",
    ),
    (
        "Door (press E)",
        "// Press E near the object to open / close it by sliding it up.\nvar closed_y = 0\nvar open = false\n\non start {\n    closed_y = position.y\n}\n\non update {\n    if pressed(\"e\") and distance(self, player) < 3 {\n        open = !open\n    }\n    var target = closed_y\n    if open {\n        target = closed_y + 3\n    }\n    position.y = lerp(position.y, target, 5 * dt)\n}\n",
    ),
    (
        "Pickup (set Collision = Trigger)",
        "// Set this object's Collision to Trigger (Inspector > Collision).\n// It disappears when the player walks into it.\non touch {\n    print(\"picked up \" + self.name)\n    visible = false\n}\n",
    ),
    (
        "Follow the player",
        "// Slowly walks toward the player.\nvar speed = 2\n\non update {\n    var d = distance(self, player)\n    if d > 2 {\n        position.x += (player.x - position.x) / d * speed * dt\n        position.z += (player.z - position.z) / d * speed * dt\n    }\n}\n",
    ),
    (
        "Respawn (press R)",
        "// Put this on any object: R sends the player back up to the start.\non key(\"r\") {\n    player.x = 0\n    player.y = 5\n    player.z = 0\n}\n",
    ),
    (
        "Every 2 seconds",
        "// Changes color every 2 seconds.\non every(2) {\n    color.r = random()\n    color.g = random()\n    color.b = random()\n}\n",
    ),
    (
        "Coin counter (HUD)",
        "// Put this on a coin (set Collision = Trigger). All coins share the number \"coins\".\non touch {\n    add(\"coins\", 1)\n    hud(\"Coins: \" + get(\"coins\"))\n    destroy()\n}\n",
    ),
    (
        "Jump pad (set Collision = Trigger)",
        "// Throws the player 12 units into the air when they step on it.\non touch {\n    launch(12)\n}\n",
    ),
    (
        "GUI: score, health bar, menu button",
        "// Make three elements in View > Game GUI Editor:
//   a Text called \"Score\", a Bar called \"Health\" and a Button called \"Menu\".
var coins = 0
var hp = 1

on start {
    gui_text(\"Score\", \"Coins: 0\")
    gui_value(\"Health\", hp)
}

on update {
    if pressed(\"e\") {
        coins += 1
        gui_text(\"Score\", \"Coins: \" + coins)
    }
    if pressed(\"h\") {
        hp = clamp(hp - 0.1, 0, 1)
        gui_value(\"Health\", hp)
        gui_color(\"Health\", 1 - hp, hp, 0.2)
    }
    // Tab frees the mouse for the button; click it or press Tab again
    if pressed(\"tab\") {
        cursor(true)
    }
    if clicked(\"Menu\") {
        cursor(false)
        print(\"menu clicked\")
    }
}
",
    ),
];
