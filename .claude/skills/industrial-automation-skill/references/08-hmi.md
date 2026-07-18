# HMI — generating operator screens

A screen is one JSON document under the project's `hmi/` directory,
addressed by slug like a POU (`cs hmi get overview`). You author it the way
you author everything else in IA2: through the CLI against the running
server, with the IDE reflecting every change live. The intended rhythm is
**generate a baseline, then reshape it element by element** — each `cs hmi
op` batch renders immediately in any open canvas with a brief spawn
animation on exactly the nodes you touched, so the human watches the screen
assemble as you work. Prefer many small op batches over one big save: the
live feedback is the point, and atomic batches keep every intermediate
state valid.

## The workflow

Start from project truth, not from a blank page. `cs hmi generate overview`
builds a deterministic first pass — alarm bar on top, one section per POU
file, BOOLs as indicators, numerics as value readouts, `*_sp`-named
numerics as setpoint inputs with confirmed write actions, and a trend over
the first numerics. It is deliberately boring and always the same for the
same project; your job is the creative pass on top of it. If the screen
already exists, generate returns 409 unless you pass `--force` — never
force over a screen a human may have curated without asking.

Then look before you edit. `cs hmi get overview` prints the document;
`cs hmi symbols` prints the palette contract (each built-in symbol's
bindable keys, props and default size); `cs hmi check overview` validates
structure and warns about bindings that name variables no POU declares.
With the picture in hand, reshape incrementally:

```bash
# move the trend, retitle the screen
echo '[{"op":"update_node","id":"trend_main","patch":{"x":24,"y":520,"w":1232}},
      {"op":"set_meta","title":"Carbonation — Overview"}]' | cs hmi op overview --from -

# add a tank wired to a level variable
echo '[{"op":"add_node","node":{"id":"tank_t201","type":"symbol","symbol":"tank",
       "x":80,"y":90,"w":140,"h":200,
       "props":{"label":"T-201","unit":"%"},
       "bind":{"value":"level_pct","alarm":"level_hh"}}}]' | cs hmi op overview --from -

# a valve the operator can command, with confirmation
echo '[{"op":"add_node","node":{"id":"valve_yv201","type":"symbol","symbol":"valve",
       "x":260,"y":250,"w":48,"h":48,"props":{"label":"YV-201"},
       "bind":{"open":"yv201_fb"},
       "action":{"tap":{"kind":"toggle","variable":"yv201_cmd","confirm":true}}}}]' \
  | cs hmi op overview --from -
```

The three op kinds beyond `add_node`: `update_node` shallow-merges a patch
(object fields like `bind`/`action`/`props` merge one level, scalars
replace, `null` deletes a key; `id` and `type` are immutable — remove and
re-add to change a node's type), `remove_node` deletes a node and its
subtree, `set_meta` edits title/level/grid. A batch is atomic: one bad op
rejects the whole batch with a message naming the op index, and the
document on disk is untouched.

Close the loop visually. After a few batches, render the screen and look
at it the way the Pencil workflow looks at a canvas: open
`http://127.0.0.1:3001/?project=NAME&hmi=overview` (or the vite dev port)
headless, screenshot, and judge your own layout — overlaps, crowding,
reading order, whether color is carrying state or decoration. Iterate with
more ops. When the program is running, bindings go live and the screenshot
shows real values, which is the strongest self-review available.

## The document model, briefly

Nodes are a closed set: `group` (absolute or simple flows), `text`,
`value`, `symbol`, `trend`, `alarmbar`, `button`, `input`, `nav`, `shape`.
Coordinates live on a fixed grid (default 1280×800, snap 8) that every
client letterboxes identically. `bind` maps a prop to a variable — a bare
name in the common case, or `{"variable":"x_raw","expr":"x / 100",
"format":"%.1f"}` for scaling; names resolve exactly like the Monitor's,
including `instance.variable` on multi-PROGRAM runs. Expressions see only
the single bound value `x` on purpose — logic that spans variables belongs
in a POU, not hidden in a screen. `action` is the only write path
(`write` / `toggle` / `pulse` / `set_value` / `nav`), every action is
declared in the reviewable document, and `confirm` defaults to true —
leave it on for anything that moves the plant.

## Style: high-performance HMI, already built in

The built-in symbols implement ISA-101 on IA2's design tokens, so a
generated screen is compliant by construction: calm gray-on-warm-neutral
in the normal state, color only when it means something (running/healthy
green, attention ochre, fault red). Preserve that when you compose —
resist decorating. One alarm bar per screen at the top; group by process
area, reading left-to-right in flow order; use `level` honestly (1 plant
overview → 4 diagnostic detail) and `nav` nodes to descend, rather than
cramming levels together.
