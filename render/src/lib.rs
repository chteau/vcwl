//! VCWL's renderer. Applications build an element tree through the command buffer; the
//! renderer computes styles and layout itself, paints into a display list that the host
//! replays on a `<canvas>`, and turns pointer and keyboard input into application events.
//! The browser DOM is not used. The only browser service it calls is text measurement.

pub mod anim;
pub mod css;
pub mod dom;
pub mod engine;
pub mod layout;
pub mod paint;
pub mod style;
pub mod text;

#[cfg(target_arch = "wasm32")]
mod wasm {
    //! WebAssembly interface. Strings from the host are written into the buffer returned by
    //! `alloc`; results are read from `out_ptr()` after a call returns their length.

    use std::cell::RefCell;

    use crate::engine::Engine;
    use crate::text::Measure;

    #[link(wasm_import_module = "host")]
    unsafe extern "C" {
        fn measure(
            font: *const u8,
            font_len: usize,
            spacing: f32,
            text: *const u8,
            text_len: usize,
        ) -> f32;
    }

    struct Host;

    impl Measure for Host {
        fn width(&mut self, font: &str, spacing: f32, text: &str) -> f32 {
            unsafe {
                measure(
                    font.as_ptr(),
                    font.len(),
                    spacing,
                    text.as_ptr(),
                    text.len(),
                )
            }
        }
    }

    thread_local! {
        static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
        static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static PANIC: RefCell<String> = const { RefCell::new(String::new()) };
    }

    fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
        ENGINE.with(|e| {
            f(e.borrow_mut()
                .as_mut()
                .expect("init() must be called first"))
        })
    }

    fn input(len: usize) -> String {
        INPUT.with(|b| String::from_utf8_lossy(&b.borrow()[..len]).into_owned())
    }

    fn output(bytes: Vec<u8>) -> usize {
        let n = bytes.len();
        OUTPUT.with(|o| *o.borrow_mut() = bytes);
        n
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: usize) -> *mut u8 {
        INPUT.with(|b| {
            let mut b = b.borrow_mut();
            b.resize(len, 0);
            b.as_mut_ptr()
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn out_ptr() -> *const u8 {
        OUTPUT.with(|o| o.borrow().as_ptr())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init(styles_len: usize, width: f32, height: f32) {
        std::panic::set_hook(Box::new(|info| {
            PANIC.with(|p| *p.borrow_mut() = info.to_string())
        }));
        let styles = input(styles_len);
        ENGINE.with(|e| *e.borrow_mut() = Some(Engine::new(&styles, width, height)));
    }

    /// Message of the last panic (the renderer traps after panicking).
    #[unsafe(no_mangle)]
    pub extern "C" fn panic_message() -> usize {
        output(PANIC.with(|p| p.borrow().clone().into_bytes()))
    }

    /// Applies `len` bytes of commands; returns 0, or the length of an error message.
    #[unsafe(no_mangle)]
    pub extern "C" fn apply(len: usize) -> usize {
        let r = INPUT.with(|b| with(|e| e.apply(&b.borrow()[..len])));
        match r {
            Ok(()) => 0,
            Err(msg) => output(msg.into_bytes()),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn resize(width: f32, height: f32) {
        with(|e| e.resize(width, height));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn preferences(dark: i32, reduced_motion: i32) {
        with(|e| e.set_preferences(dark != 0, reduced_motion != 0));
    }

    /// Paints if needed; returns the display list length in bytes (0 if unchanged).
    #[unsafe(no_mangle)]
    pub extern "C" fn frame(now: f64) -> usize {
        match with(|e| e.frame(&mut Host, now)) {
            Some(words) => output(words.iter().flat_map(|w| w.to_le_bytes()).collect()),
            None => 0,
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn next_wake(now: f64) -> f64 {
        with(|e| e.next_wake(now))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn dirty() -> i32 {
        with(|e| e.dirty() as i32)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pointer_down(x: f32, y: f32, touch: i32, now: f64) {
        with(|e| {
            e.now = now;
            e.pointer_down(&mut Host, x, y, touch != 0)
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pointer_move(x: f32, y: f32) {
        with(|e| e.pointer_move(&mut Host, x, y));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pointer_up(x: f32, y: f32, now: f64) {
        with(|e| {
            e.now = now;
            e.pointer_up(&mut Host, x, y)
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn wheel(x: f32, y: f32, dx: f32, dy: f32) {
        with(|e| e.wheel(x, y, dx, dy));
    }

    /// Handles a key (name in the input buffer); returns 1 if consumed.
    #[unsafe(no_mangle)]
    pub extern "C" fn key(len: usize, shift: i32, ctrl: i32, alt: i32, now: f64) -> i32 {
        let k = input(len);
        with(|e| {
            e.now = now;
            e.key(&mut Host, &k, shift != 0, ctrl != 0, alt != 0) as i32
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn insert_text(len: usize) {
        let s = input(len);
        with(|e| e.insert_text(&s));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn enter(backward: i32) {
        with(|e| e.enter(backward != 0));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn leave() {
        with(|e| e.leave());
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn image_ready(id: u32, w: f32, h: f32) {
        with(|e| e.image_ready(id, w, h));
    }

    /// Next batch of host work (see `Engine::poll`); returns its length in bytes.
    #[unsafe(no_mangle)]
    pub extern "C" fn poll() -> usize {
        output(with(|e| e.poll()))
    }

    /// The accessibility mirror if it changed (see `Engine::semantics`); 0 otherwise.
    #[unsafe(no_mangle)]
    pub extern "C" fn semantics() -> usize {
        with(|e| e.semantics(&mut Host)).map_or(0, output)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn focus_state() -> usize {
        output(with(|e| e.focus_state()))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn focus_node(id: u32) {
        with(|e| e.focus_node(id));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn activate_node(id: u32) {
        with(|e| e.activate_node(id));
    }

    /// A mirrored text field changed: value in the input buffer, selection in UTF-16 units.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_input(id: u32, len: usize, start: i32, end: i32, backward: i32) {
        let v = input(len);
        with(|e| e.field_input(id, &v, start, end, backward != 0));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn scroll_to_top() {
        with(|e| e.scroll_to_top());
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_keydown(len: usize) {
        let k = input(len);
        with(|e| e.field_keydown(&k));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn choose_index(id: u32, i: u32) {
        with(|e| e.choose_index(id, i as usize));
    }

    /// Style rules, a NUL byte, then the tree as HTML (prerendering).
    #[unsafe(no_mangle)]
    pub extern "C" fn prerender() -> usize {
        let (html, css) = with(|e| e.html());
        output(format!("{css}\0{html}").into_bytes())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn fonts_changed() {
        with(|e| e.fonts_changed());
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn take_over(scroll_y: f32) {
        with(|e| e.take_over(scroll_y));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn dump() -> usize {
        output(with(|e| e.dump()).into_bytes())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn live_nodes() -> usize {
        with(|e| e.tree.live())
    }

    /// Commands applied (0) and moves of attached nodes (1) since start.
    #[unsafe(no_mangle)]
    pub extern "C" fn counter(which: i32) -> usize {
        with(|e| {
            if which == 0 {
                e.tree.commands
            } else {
                e.tree.moves
            }
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn live_listeners() -> usize {
        with(|e| {
            e.tree
                .nodes
                .iter()
                .flatten()
                .map(|n| n.listeners.len())
                .sum()
        })
    }
}
