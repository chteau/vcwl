# 15. Built-in 3D scenes as elements, drawn by a lazily loaded WebGL module

Status: accepted

## Context

Apps wanted 3D content (product views, visualizations) like three.js offers, declared in the
same language and reactive to state like the rest of the view.

## Decision

- `scene` with `mesh`, `group` and `light` children are elements checked by the compiler (their
  arguments are typed; 3D elements are only valid inside a scene and only they are). They live in
  the renderer's tree, so reactivity, events and keyed lists need nothing new.
- The renderer lays out and clips the scene's box and emits a `SCENE` paint op; it exports
  `scene(id)` describing the content. The 3D drawing itself is JavaScript (`scene3d.js`, WebGL 1,
  Blinn-Phong shading, procedural geometry), loaded only by apps that show a scene.
- The WebGL output is copied into the 2D page canvas, so scenes scroll, clip and stack like other
  content. Clicks are resolved with a GPU pick pass and delivered as the mesh's `click`.

Alternatives: three.js would bring loaders, materials and shadows, but is a large third-party
library and a second scene graph to synchronize; drawing 3D in Rust inside render.wasm would put
WebGL calls behind a host bridge for every draw and grow the module every app downloads.

## Consequences

- Apps without scenes pay nothing (the module is not fetched; render.wasm grows by the op and
  the description only).
- The copy from WebGL to the 2D canvas costs a full-scene blit per frame while a scene animates.
- Models are loaded by `scene3d.js` itself (OBJ, and glTF with each material's base color and
  base color texture), textures as images;
  one shadow map (RGBA-packed depth, so no WebGL extension is needed). No materials, skinning
  or animation from files.
- Clickable meshes need a `label` and are focusable buttons in the accessibility mirror,
  placed over the scene (a mesh has no box of its own, so it takes its scene's); the rest of
  the scene is visible to assistive technology only through its `label`.
