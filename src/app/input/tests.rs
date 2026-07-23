//! src/app/input/tests.rs — the click/drag-selection unit-test suite
//! (formerly `mod click_tests`), moved verbatim out of the former
//! `app/input.rs` monolith (2026-07 code-organization pass) and renamed
//! to the directory-split convention's plain `tests` — every test's
//! behavior is unchanged, only its module path
//! (`app::input::click_tests::foo` -> `app::input::tests::foo`, no
//! external caller named the old path).

use crate::app::*;
use crate::render::{Metrics, TEXT_LEFT, TEXT_TOP};

// Every `App` below is built via `App::new_hermetic` (see its doc on
// `App::new` in `app.rs`) — these tests only care about click/selection
// behavior over a `set_text` fixture, never real file content, so the
// hermetic constructor's injected `InMemoryFs` + disabled session-restore
// keep them from ever touching the developer's real
// `~/.local/share/awl/{session.toml,scratch.md}`.

/// Place a synthetic press at document (line 0, `col`) — the GPU-less
/// `hit_test_char` fallback path (`render::hit_test` with fixed-pitch
/// `Metrics`), so this drives the exact same math a real click does.
fn press_at_col(app: &mut App, col: usize, shift: bool) {
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    app.cursor_px = (TEXT_LEFT + col as f32 * m.char_width, TEXT_TOP);
    app.on_press(shift, true);
}

#[test]
fn gutter_press_never_moves_or_selects_document_text() {
    // THE REPORTED BUG: margin/gutter coordinates used to enter the ordinary
    // hit-test, whose correct out-of-range clamp made the click land on the page's
    // first text column. The writing-column gate must leave the entire edit state
    // untouched — cursor, active selection, and drag arm alike.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    app.buffer.select_range(2, 7);
    let before_cursor = app.buffer.cursor_char();
    let before_selection = app.buffer.selection_range();

    app.cursor_px = (0.0, TEXT_TOP);
    app.on_press(false, false);

    assert_eq!(
        app.buffer.cursor_char(),
        before_cursor,
        "gutter press leaves the caret alone"
    );
    assert_eq!(
        app.buffer.selection_range(),
        before_selection,
        "gutter press neither creates nor clears a document selection"
    );
    assert!(
        !app.dragging,
        "gutter press cannot arm a text-selection drag"
    );
    assert!(
        !app.drag_armed,
        "gutter press cannot cross the drag-slop gate later"
    );

    // Moving well past the normal drag slop after that ignored press remains a
    // no-op: the gutter cannot become a delayed selection start.
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    move_by(&mut app, m.char_width * 4.0, 0.0);
    assert_eq!(app.buffer.cursor_char(), before_cursor);
    assert_eq!(app.buffer.selection_range(), before_selection);
}

#[test]
fn plain_click_clears_the_mark_and_places_the_cursor() {
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    app.buffer.set_cursor(0);
    app.buffer.set_mark(); // an existing selection from a prior gesture
    press_at_col(&mut app, 6, false); // "w" of "world"
    assert!(
        !app.buffer.has_selection(),
        "a plain click drops any selection"
    );
    assert_eq!(app.buffer.cursor_char(), 6);
}

#[test]
fn shift_click_extends_from_the_cursors_prior_position() {
    // No existing mark: a shift-click must DROP the mark at wherever the
    // cursor already sat (char 0), then move ONLY the cursor to the hit
    // point — never `clear_mark`.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    app.buffer.set_cursor(0);
    assert!(app.buffer.anchor_char().is_none());
    press_at_col(&mut app, 6, true);
    assert_eq!(
        app.buffer.anchor_char(),
        Some(0),
        "mark drops at the prior cursor spot"
    );
    assert_eq!(app.buffer.cursor_char(), 6, "cursor moves to the click");
    assert_eq!(app.buffer.selection_range(), Some((0, 6)));
}

#[test]
fn shift_click_keeps_an_already_active_mark() {
    // A mark is already active (e.g. from C-Space or a prior shift-click):
    // a further shift-click must NOT move the mark, only the cursor.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    app.buffer.set_cursor(2);
    app.buffer.set_anchor(1); // mark pinned at char 1
    press_at_col(&mut app, 9, true);
    assert_eq!(
        app.buffer.anchor_char(),
        Some(1),
        "an active mark is never disturbed"
    );
    assert_eq!(app.buffer.cursor_char(), 9);
}

#[test]
fn double_and_triple_click_arms_ignore_shift() {
    // The word/line-select arms (click_count 2/3) are untouched by shift —
    // shift only modifies the single-click arm.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    // A first click at col 0 primes the multi-click detector; the SECOND
    // press at the same spot (inside `on_press`'s own `bump_click_count`
    // call) is recognized as the double-click, exactly as two real clicks
    // would be.
    press_at_col(&mut app, 0, false);
    press_at_col(&mut app, 0, true);
    // A double click at col 0 still selects the word "hello" wholesale,
    // exactly as an un-shifted double click would.
    assert_eq!(app.buffer.selection_range(), Some((0, 5)));
}

// === THE PHANTOM-SELECTION-CLICK FIX ================================
// `App::drag_armed` / `App::exceeds_drag_slop`: a `CursorMoved` while
// `dragging` must only extend the selection once the pointer has genuinely
// traveled past `DRAG_ARM_SLOP_PX` from the press position — never merely
// because a WYSIWYG reveal reflow (concealed markup regaining its real glyph
// advance the instant the caret lands on that line) shifted what the SAME
// pixel position would now hit-test to. The pure `exceeds_drag_slop`
// geometry check below proves the arm decision reads pixel travel alone;
// the `App`-level tests prove the wiring end to end over the real
// `on_press` / `on_cursor_moved` seam.

#[test]
fn exceeds_drag_slop_is_false_for_a_perfectly_stationary_pointer() {
    // THE CORE OF THE FIX: zero pixel travel never arms a drag, no matter
    // what a hit-test at that same position would now resolve to (a reveal
    // reflow changes the hit-test RESULT, never the pointer's own pixel
    // position) — `exceeds_drag_slop` only ever looks at the two positions.
    assert!(!App::exceeds_drag_slop((100.0, 200.0), (100.0, 200.0)));
}

#[test]
fn exceeds_drag_slop_is_false_for_sub_slop_jitter() {
    // Real mice/trackpads report tiny (sub-pixel-rounded) motion even while
    // "held still" — e.g. the physical act of pressing the button. Anything
    // strictly under the slop must not arm.
    assert!(!App::exceeds_drag_slop((100.0, 200.0), (102.0, 200.0)));
    assert!(!App::exceeds_drag_slop((100.0, 200.0), (100.0, 203.0)));
    // Right at the threshold (distance == slop, not >) still does not arm —
    // the comparison is strict `>`.
    assert!(!App::exceeds_drag_slop((0.0, 0.0), (DRAG_ARM_SLOP_PX, 0.0)));
}

#[test]
fn exceeds_drag_slop_is_true_past_the_threshold() {
    assert!(App::exceeds_drag_slop((100.0, 200.0), (105.0, 200.0)));
    assert!(App::exceeds_drag_slop((100.0, 200.0), (100.0, 205.0)));
}

#[test]
fn exceeds_drag_slop_combines_both_axes_diagonally() {
    // Neither axis alone clears the slop, but the diagonal (Euclidean)
    // distance does — the squared-distance compare must sum both axes, not
    // check them independently.
    let (dx, dy): (f32, f32) = (3.0, 3.0);
    assert!(
        (dx * dx + dy * dy).sqrt() > DRAG_ARM_SLOP_PX,
        "test fixture sanity"
    );
    assert!(App::exceeds_drag_slop((0.0, 0.0), (dx, dy)));
}

/// Move the live pointer by a pixel delta from its CURRENT `cursor_px` and
/// drive it through the real `on_cursor_moved` seam — the same path a real
/// `WindowEvent::CursorMoved` takes.
fn move_by(app: &mut App, dx: f32, dy: f32) {
    let (x, y) = app.cursor_px;
    app.on_cursor_moved(winit::dpi::PhysicalPosition::new(
        (x + dx) as f64,
        (y + dy) as f64,
    ));
}

#[test]
fn stationary_pointer_after_press_never_arms_a_selection() {
    // A press, then a `CursorMoved` reporting the EXACT press pixel again —
    // exactly what a reveal-reflow's redraw could look like if it ever
    // spuriously re-delivered the pointer position (or a genuinely idle
    // pointer between press and release) — must read as a plain click.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    press_at_col(&mut app, 6, false);
    assert_eq!(app.buffer.cursor_char(), 6);
    move_by(&mut app, 0.0, 0.0);
    assert!(
        !app.buffer.has_selection(),
        "no travel must never arm a selection"
    );
    assert_eq!(
        app.buffer.cursor_char(),
        6,
        "the caret stays at the press's own hit-test result"
    );
}

#[test]
fn sub_slop_jitter_does_not_arm_a_selection_even_across_a_column_boundary() {
    // Engineer the press to sit just BEFORE a column's rounding boundary, so
    // a jitter of less than `DRAG_ARM_SLOP_PX` is enough to make a fresh
    // hit-test resolve to the NEXT column over — standing in for a WYSIWYG
    // reveal reflow relocating the same document position by a few px under
    // an otherwise-still pointer. The fix must gate on the pointer's own
    // travel, not on whatever the hit-test now returns.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    // Half a cell short of column 6's boundary: rounds to column 6 today,
    // but a nudge of less than half a cell tips it to column 7.
    app.cursor_px = (TEXT_LEFT + 6.0 * m.char_width - 0.5, TEXT_TOP);
    app.on_press(false, true);
    let pressed_at = app.buffer.cursor_char();
    assert!(
        DRAG_ARM_SLOP_PX < m.char_width / 2.0,
        "test fixture sanity: slop < half a cell"
    );
    move_by(&mut app, DRAG_ARM_SLOP_PX - 0.1, 0.0);
    assert!(
        !app.buffer.has_selection(),
        "sub-slop travel must never arm a selection"
    );
    assert_eq!(
        app.buffer.cursor_char(),
        pressed_at,
        "the caret must not drift under sub-slop jitter"
    );
}

#[test]
fn real_drag_past_the_slop_arms_and_extends_the_selection() {
    // A genuine drag — well past the slop — must still work exactly as
    // before: the selection extends live, char by char, as the pointer
    // moves.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    press_at_col(&mut app, 0, false);
    assert!(!app.buffer.has_selection());
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    move_by(&mut app, 6.0 * m.char_width, 0.0);
    assert!(
        app.buffer.has_selection(),
        "travel past the slop must arm a real drag"
    );
    assert_eq!(app.buffer.selection_range(), Some((0, 6)));
}

#[test]
fn once_armed_a_drag_stays_armed_through_further_sub_slop_moves() {
    // A real drag that then pauses/jitters mid-gesture must keep extending
    // (armed is sticky for the rest of the gesture) — only the FIRST move of
    // a fresh press is slop-gated.
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    press_at_col(&mut app, 0, false);
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    move_by(&mut app, 6.0 * m.char_width, 0.0); // arms the drag
    assert_eq!(app.buffer.selection_range(), Some((0, 6)));
    // A tiny further nudge (well under the slop) still extends, because the
    // gesture is already armed.
    move_by(&mut app, 1.0, 0.0);
    assert!(
        app.buffer.has_selection(),
        "an already-armed drag keeps extending on any move"
    );
}

#[test]
fn release_disarms_so_the_next_press_is_slop_gated_again() {
    // The armed flag must not leak across gestures: after a real drag then
    // release, a FRESH press elsewhere followed by a sub-slop move must not
    // arm — proves `drag_armed` resets per press (belt-and-braces with the
    // release-time reset).
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text("hello world");
    press_at_col(&mut app, 0, false);
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    move_by(&mut app, 6.0 * m.char_width, 0.0);
    assert!(app.buffer.has_selection());
    app.dragging = false;
    app.drag_armed = false; // mirrors `on_mouse_input`'s Released arm
    press_at_col(&mut app, 3, false);
    assert!(
        !app.buffer.has_selection(),
        "a fresh plain click drops the old selection"
    );
    move_by(&mut app, DRAG_ARM_SLOP_PX - 0.1, 0.0);
    assert!(
        !app.buffer.has_selection(),
        "the new gesture is slop-gated again, not still armed"
    );
}

// --- ITEM 47c: click-to-expand on a collapsed heading's tail / chevron ---------
//
// The hermetic (GPU-less) press path drives `render::hit_test`'s fixed-pitch
// fallback — the exact math a real click runs — so a synthesized press at the tail
// region exercises the true affordance seam. A pathless hermetic buffer is markdown
// (so headings + folds are live), and folding parks the caret on the heading.

/// A small nested-free markdown doc (no soft-wrap): row 0 # A hides a1,a2 when
/// folded; # B / b1 stay visible.
const FOLD_DOC: &str = "# A\na1\na2\n# B\nb1";

/// Synthesize a press at a VISUAL row + char column (fixed-pitch fallback geometry),
/// landing mid-row so the y floors to `row`. Drives the real `on_press`.
fn press_at_row_col(app: &mut App, row: usize, col: usize, shift: bool) {
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    app.cursor_px = (
        TEXT_LEFT + col as f32 * m.char_width,
        TEXT_TOP + (row as f32 + 0.5) * m.line_height,
    );
    app.on_press(shift, true);
}

fn folded_app() -> App {
    let mut app = App::new_hermetic(None, PathBuf::from("/tmp"), Config::empty());
    app.buffer.set_text(FOLD_DOC);
    app.buffer.set_cursor(0); // on # A
    app.buffer.toggle_fold_at_cursor(); // fold # A -> hides a1,a2 (filtered: 0 # A / 1 # B / 2 b1)
    assert!(app.buffer.folds().contains(&0), "precondition: # A is folded");
    app
}

#[test]
fn clicking_the_collapsed_heading_tail_expands_the_fold() {
    let mut app = folded_app();
    // Press PAST the heading text on its own row (col 20 ≫ "# A".len() == 3): the
    // "… N lines" tail / chevron region. It expands the fold, parks the caret on the
    // heading, and starts NO drag.
    press_at_row_col(&mut app, 0, 20, false);
    assert!(
        app.buffer.folds().is_empty(),
        "a click on the tail affordance expanded the fold"
    );
    assert_eq!(app.buffer.cursor_line_col().0, 0, "caret parked on the heading");
    assert!(!app.dragging, "an expand click never starts a text-selection drag");
    assert!(!app.buffer.has_selection());
}

#[test]
fn clicking_the_heading_text_places_the_caret_without_expanding() {
    let mut app = folded_app();
    // Press ON the heading text (col 1, inside "# A"): the caret lands, the fold STAYS
    // collapsed — the affordance is the tail region past the text, not the text itself.
    press_at_row_col(&mut app, 0, 1, false);
    assert!(
        app.buffer.folds().contains(&0),
        "clicking the heading text does not expand the fold"
    );
    assert_eq!(app.buffer.cursor_line_col().0, 0, "caret is on the heading line");
}

#[test]
fn a_shift_click_on_the_tail_never_expands() {
    let mut app = folded_app();
    // Shift-click is a selection-extend gesture everywhere; it must not hijack into an
    // expand, even over the tail region.
    press_at_row_col(&mut app, 0, 20, true);
    assert!(
        app.buffer.folds().contains(&0),
        "a shift-click extends a selection, it never expands a fold"
    );
}

#[test]
fn a_heading_jump_onto_a_hidden_line_reveals_its_fold() {
    // REVEALED PLACEMENT (folds): a Go-to-heading / margin-outline jump targeting a
    // line hidden inside a collapsed section must reveal it — the caret can never be
    // left logically inside a fold. `jump_to_line` routes through the placement owner.
    let mut app = folded_app(); // # A folded, hiding a1 (line 1) and a2 (line 2)
    app.jump_to_line(1); // jump onto the hidden a1
    assert!(
        app.buffer.folds().is_empty(),
        "a jump onto a hidden line revealed the fold"
    );
    assert_eq!(app.buffer.cursor_line_col().0, 1, "caret parked on the now-visible line");
}

#[test]
fn a_shift_click_across_a_collapsed_section_reveals_the_fold_it_spans() {
    // THE DRAG/SHIFT-CLICK REVEAL (Wave-4 neighbourhood): with # A folded, the caret
    // parks on # A (char 0). A shift-click on the b1 row (filtered row 2 -> full line
    // 4) drops the mark at char 0 and moves the cursor to line 4 — a selection that
    // spans the hidden a1/a2. It must never span a fold invisibly: the placement
    // owner reveals # A before the selection is shown.
    let mut app = folded_app();
    assert_eq!(app.buffer.cursor_char(), 0, "precondition: caret on # A");
    press_at_row_col(&mut app, 2, 0, true);
    assert!(
        app.buffer.folds().is_empty(),
        "a shift-click whose selection spans the fold reveals it"
    );
    assert!(app.buffer.has_selection(), "the shift-click built a selection");
    let (start, end) = app.buffer.selection_range().unwrap();
    assert_eq!(start, 0, "mark stayed at the prior caret");
    // The far endpoint is the START of b1 — full line 4, now that the fold is open.
    assert_eq!(end, app.buffer.line_col_to_char(4, 0), "selection reaches b1");
}

#[test]
fn a_drag_across_a_collapsed_section_reveals_every_fold_it_crosses() {
    // A plain press on # A (its heading text, not the tail) places the caret and arms
    // a char drag; dragging DOWN past the hidden a1/a2 onto b1 must reveal # A so the
    // growing selection never crosses hidden lines. Drives the real
    // on_press -> on_cursor_moved(drag) seam.
    let mut app = folded_app();
    press_at_row_col(&mut app, 0, 0, false); // caret on # A, drag armed on next travel
    assert!(app.dragging, "a press on the heading text arms a text drag");
    assert!(app.buffer.folds().contains(&0), "the press alone does not reveal");
    let m = Metrics::with_dpi(app.zoom, app.dpi);
    // Travel two visible rows down (well past the slop) onto the b1 row.
    move_by(&mut app, 0.0, 2.0 * m.line_height);
    assert!(
        app.buffer.folds().is_empty(),
        "the drag crossing the fold revealed it"
    );
    assert!(app.buffer.has_selection(), "the drag extended a real selection");
    assert_eq!(
        app.buffer.selection_range().unwrap(),
        (0, app.buffer.line_col_to_char(4, 0)),
        "selection runs from # A to b1 with nothing hidden inside it"
    );
}

#[test]
fn a_click_below_a_collapsed_section_lands_on_the_right_full_document_line() {
    // THE FOLD HIT-TEST REMAP: with # A folded, the render shapes "# A\n# B\nb1", so a
    // click on the 2nd VISIBLE row hit-tests to filtered line 1 — which must resolve to
    // FULL-document line 3 (# B), not rope line 1 (a1, hidden inside the fold). Without
    // the visible→full remap the caret would land on the wrong (hidden) line.
    let mut app = folded_app();
    press_at_row_col(&mut app, 1, 0, false);
    assert_eq!(
        app.buffer.cursor_line_col().0,
        3,
        "click on filtered row 1 lands on full line 3 (# B), not the hidden a1"
    );
    assert!(app.buffer.folds().contains(&0), "clicking # B does not disturb the fold");
}
