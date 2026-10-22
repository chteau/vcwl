# 11. An invisible DOM mirror for accessibility and text input

Status: accepted (amends [ADR 10](0010-canvas-renderer.md))

## Context

With the canvas renderer, screen readers saw one canvas, text fields had no input method (IME)
composition or mobile virtual keyboard, and search engines saw an empty page. A branding website
needs all three.

Options considered:

1. Keep the canvas only and implement platform accessibility APIs ourselves: not possible from a
   web page; browsers expose accessibility only through the DOM (and the unfinished AOM).
2. Render the real DOM again for semantics, styled invisibly: this is what Flutter's web semantics
   tree does. It costs DOM work proportional to the page.
3. Mirror only focusable controls: cheap, but headings, landmarks, lists and text would be missing.

## Decision

Option 2, limited to semantic elements. The renderer (`render/src/engine/semantics.rs`) produces a
flat pre-order list of the elements whose tag has meaning (headings, paragraphs, links, buttons,
form controls, landmarks, lists, tables, `img` as `role="img"`) or that carry a `role`, plus
text runs, each with its viewport rectangle, states and `aria-*` attributes. Generic containers
are skipped. Each entry carries a hash so the host skips unchanged ones.

The host (`runtime/vcwl.js`) keeps one transparent, `position: fixed` element per entry, placed
exactly over what the canvas draws, under a container with `pointer-events: none`; the canvas is
`aria-hidden`. The mirror is brought up to date right after each canvas paint, reordering with a
longest-increasing-subsequence pass so a swap moves two elements, not the whole list.

- Keyboard focus lives on mirror elements. The renderer stays in charge of focus order and
  activation; the host moves DOM focus to match and forwards focus that arrives from outside
  (Tab from the address bar, a screen reader) with `focus_node`.
- Activation by assistive technology arrives as a `click` on a mirror element and is forwarded
  with `activate_node`. Pointer input still goes to the canvas.
- Text fields are edited natively by the mirror `<input>`/`<textarea>`, which gives IME, mobile
  keyboards, autocorrect, paste and undo. Each `input` and `selectionchange` sends the value and
  selection (UTF-16 offsets) to the renderer (`field_input`), which draws the caret. Selections
  the renderer makes (clicks, drags) and values the app sets go back to the DOM at once.

## Consequences

- Screen readers get headings with levels, landmarks, link and button names, form labels and
  states; puppeteer's accessibility snapshot is used in tests to check this.
- The DOM is back, but only for semantics: about 20 ms to build the mirror for 1000 list rows and
  1.5–4 ms per update in the rows benchmark, after the canvas has been drawn.
- Reading order is tree order. Screen-reader browse mode does not scroll the canvas; focusing a
  control does.
- Crawlers that run JavaScript see the mirror's text and links.
