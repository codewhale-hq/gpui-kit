//! Right-click the real styled frame/editor; do not call their event handlers.
use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui_kit::{
    App, AppContext, ClipboardItem, Context, Entity, Focusable, InputEvent as _, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext,
    Window, WindowHandle,
    base::{Root, input::InputContextMenuCapabilities},
    component::input::{Copy, Cut, Escape, Input, InputState},
    div, point,
    prelude::*,
    px, size,
    test::{TestSupportExt, TestWindowExt},
};

use crate::common;

#[cfg(target_os = "macos")]
const SELECT_ALL: &str = "cmd-a";
#[cfg(not(target_os = "macos"))]
const SELECT_ALL: &str = "ctrl-a";

#[derive(Debug)]
struct Request {
    selection: Range<usize>,
    cursor: usize,
    capabilities: InputContextMenuCapabilities,
}

struct Fields {
    input: Entity<InputState>,
    other: Entity<InputState>,
    requests: Rc<RefCell<Vec<Request>>>,
    disabled: bool,
    readonly: bool,
    recording: bool,
    release_change: Option<ReleaseChange>,
    delivered_capabilities: Rc<RefCell<Vec<InputContextMenuCapabilities>>>,
}

#[derive(Clone, Copy)]
enum ReleaseChange {
    Disable,
    OptOut,
    Mask,
    Blur,
}

impl Render for Fields {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let input = self.input.clone();
        let requests = self.requests.clone();
        let release_input = self.input.clone();
        let release_change = self.release_change;
        let delivered_capabilities = self.delivered_capabilities.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_4()
            // An ancestor may consume Escape before raw key listeners run.
            .on_action(|_: &Escape, _, _| {})
            .child(
                Input::new(&self.input)
                    .id("field")
                    .w(px(320.))
                    .disabled(self.disabled)
                    .readonly(self.readonly)
                    .suffix(
                        div()
                            .id("owned-suffix")
                            .test_support()
                            .w_8()
                            .h_6()
                            .on_mouse_down(MouseButton::Right, |_, window, _| {
                                window.prevent_default()
                            })
                            .on_mouse_up(MouseButton::Right, move |_, window, cx| {
                                // A real descendant bubble handler runs after the
                                // frame's capture-up, before deferred delivery.
                                match release_change {
                                    Some(ReleaseChange::Disable) => release_input
                                        .update(cx, |state, cx| state.set_disabled(true, cx)),
                                    Some(ReleaseChange::OptOut) => release_input
                                        .update(cx, |state, _| {
                                            state.set_context_menu_enabled(false)
                                        }),
                                    Some(ReleaseChange::Mask) => {
                                        release_input.update(cx, |state, cx| {
                                            state.set_masked(true, window, cx);
                                            let delivered = delivered_capabilities.clone();
                                            state.on_context_menu(Rc::new(
                                                move |_, capabilities, _, _, _| {
                                                    delivered.borrow_mut().push(capabilities);
                                                },
                                            ));
                                        })
                                    }
                                    Some(ReleaseChange::Blur) => window.blur(cx),
                                    None => {}
                                }
                            }),
                    )
                    .when(self.recording, |this| {
                        this.context_menu(move |menu, _, cx| {
                            let state = input.read(cx);
                            requests.borrow_mut().push(Request {
                                selection: state.selected_range(),
                                cursor: state.cursor(),
                                capabilities: state.context_menu_capabilities(),
                            });
                            // Empty native menus do not enter an OS tracking loop. This
                            // records actual pointer-to-builder delivery, not OS display.
                            menu
                        })
                    }),
            )
            .child(Input::new(&self.other).id("other").w(px(320.)))
    }
}

fn fields(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<Fields>) {
    cx.update(gpui_kit::init);
    let result = common::open_window(cx, Some(size(px(640.), px(480.))), |window, cx| {
        cx.new(|cx| Fields {
            input: cx.new(|cx| InputState::new(window, cx).default_value("abcdef")),
            other: cx.new(|cx| InputState::new(window, cx).default_value("unrelated")),
            requests: Rc::default(),
            disabled: false,
            readonly: false,
            recording: true,
            release_change: None,
            delivered_capabilities: Rc::default(),
        })
    });
    cx.update_window(result.0.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    result
}

fn padding(window: &Window, view: &Entity<Fields>, cx: &App) -> Point<Pixels> {
    let frame = window.find("field").bounds();
    let position = point(frame.left() + px(2.), frame.center().y);
    assert!(
        !view
            .read(cx)
            .input
            .read(cx)
            .input_bounds()
            .contains(&position),
        "the regression must hit frame padding, outside the editor"
    );
    position
}

fn right(window: &mut Window, position: Point<Pixels>, down: bool, cx: &mut App) {
    dispatch_right(window, position, down, cx);
    window.render_frame(cx);
}

fn dispatch_right(window: &mut Window, position: Point<Pixels>, down: bool, cx: &mut App) {
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: (!down).then_some(MouseButton::Right),
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    if down {
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
    } else {
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    }
}

fn request_count(view: &Entity<Fields>, cx: &TestAppContext) -> usize {
    view.read_with(cx, |view, _| view.requests.borrow().len())
}

#[gpui_kit::test]
fn padding_preserves_forward_and_reversed_selection_and_opens_once(cx: &mut TestAppContext) {
    let (handle, view) = fields(cx);
    for reversed in [false, true] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("field", cx);
            window.press(SELECT_ALL, cx);
            window.press(if reversed { "right" } else { "left" }, cx);
            for _ in 0..if reversed { 1 } else { 2 } {
                window.press(if reversed { "left" } else { "right" }, cx);
            }
            for _ in 0..3 {
                window.press(
                    if reversed {
                        "shift-left"
                    } else {
                        "shift-right"
                    },
                    cx,
                );
            }
            let state = view.read(cx).input.read(cx);
            assert_eq!(state.selected_range(), 2..5);
            assert_eq!(state.cursor(), if reversed { 2 } else { 5 });
            window.click("other", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let before = request_count(&view, cx);
        cx.update_window(handle.into(), |_, window, cx| {
            let pad = padding(window, &view, cx);
            right(window, pad, true, cx);
            assert_eq!(
                view.read(cx).requests.borrow().len(),
                before,
                "not on press"
            );
            right(window, pad, false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(request_count(&view, cx), before + 1);
        cx.update_window(handle.into(), |_, window, cx| {
            let field = &view.read(cx).input;
            assert!(field.focus_handle(cx).is_focused(window));
            let requests = view.read(cx).requests.borrow();
            let request = requests.last().unwrap();
            assert_eq!(request.selection, 2..5);
            assert_eq!(request.cursor, if reversed { 2 } else { 5 });
            assert!(request.capabilities.is_copyable());
            assert_eq!(view.read(cx).other.read(cx).value(), "unrelated");
        })
        .unwrap();
    }
    // The input's own right-down and the frame's capture-up share one request.
    let before = request_count(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.right_click("field", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(request_count(&view, cx), before + 1);
    view.read_with(cx, |view, _| {
        let requests = view.requests.borrow();
        let request = requests.last().unwrap();
        assert_eq!(
            request.selection,
            6..6,
            "an inner hit past the text still places the caret"
        );
        assert_eq!(request.cursor, 6);
    });
}

#[gpui_kit::test]
fn aborted_and_unmatched_releases_do_not_reopen_a_stale_request(cx: &mut TestAppContext) {
    let (handle, view) = fields(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        let pad = padding(window, &view, cx);
        right(window, pad, false, cx); // No preceding press.
        right(window, pad, true, cx);
        right(window, point(px(600.), px(440.)), false, cx);
        right(window, pad, false, cx); // The cancelled press cannot be reused.
        right(window, pad, true, cx);
        window.click("other", cx); // Another button/owner cancels it.
        right(window, pad, false, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(request_count(&view, cx), 0);
    // Either direction across padding/editor remains the same field gesture.
    for start_in_padding in [true, false] {
        cx.update_window(handle.into(), |_, window, cx| {
            let pad = padding(window, &view, cx);
            let editor = view.read(cx).input.read(cx).input_bounds().center();
            right(
                window,
                if start_in_padding { pad } else { editor },
                true,
                cx,
            );
            right(
                window,
                if start_in_padding { editor } else { pad },
                false,
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(request_count(&view, cx), 2);
    // Releasing the menu gesture must not swallow the editor's drag cleanup.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("field", cx);
        window.press(SELECT_ALL, cx);
        window.input("replacement", cx);
        assert_eq!(window.find("field").value(), Some("replacement"));
        assert_eq!(window.find("other").value(), Some("unrelated"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn optout_keeps_normal_focus_and_disabled_or_prevented_presses_do_not_steal_focus(
    cx: &mut TestAppContext,
) {
    let (handle, view) = fields(cx);
    for disabled in [false, true] {
        common::update_content(handle, &view, cx, |view, _, cx| {
            view.disabled = disabled;
            view.input
                .update(cx, |state, _| state.set_context_menu_enabled(disabled));
            cx.notify();
        })
        .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("other", cx);
            let pad = padding(window, &view, cx);
            right(window, pad, true, cx);
            right(window, pad, false, cx);
            if disabled {
                assert!(view.read(cx).other.focus_handle(cx).is_focused(window));
            } else {
                assert!(view.read(cx).input.focus_handle(cx).is_focused(window));
            }
        })
        .unwrap();
        cx.run_until_parked();
    }
    common::update_content(handle, &view, cx, |view, _, cx| {
        view.disabled = false;
        view.input
            .update(cx, |state, _| state.set_context_menu_enabled(true));
        cx.notify();
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("other", cx);
        window.right_click("owned-suffix", cx);
        assert!(view.read(cx).other.focus_handle(cx).is_focused(window));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(request_count(&view, cx), 0);
    // A stale press whose release never arrived is replaced, not duplicated.
    cx.update_window(handle.into(), |_, window, cx| {
        let pad = padding(window, &view, cx);
        right(window, pad, true, cx);
        right(window, pad, true, cx);
        right(window, pad, false, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(request_count(&view, cx), 1);
}

#[gpui_kit::test]
fn descendant_release_changes_are_rechecked_before_menu_delivery(cx: &mut TestAppContext) {
    for change in [
        ReleaseChange::Disable,
        ReleaseChange::OptOut,
        ReleaseChange::Mask,
        ReleaseChange::Blur,
    ] {
        let (handle, view) = fields(cx);
        common::update_content(handle, &view, cx, |view, _, cx| {
            view.release_change = Some(change);
            cx.notify();
        })
        .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("field", cx);
            window.press(SELECT_ALL, cx);
            assert!(
                view.read(cx)
                    .input
                    .read(cx)
                    .context_menu_capabilities()
                    .is_copyable()
            );
            let pad = padding(window, &view, cx);
            right(window, pad, true, cx);
            let suffix = window.find("owned-suffix").bounds().center();
            // Drain deferred delivery before another render replaces the
            // handler installed by the real suffix release callback.
            dispatch_right(window, suffix, false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(request_count(&view, cx), 0);
        view.read_with(cx, |view, _| {
            let delivered = view.delivered_capabilities.borrow();
            if matches!(change, ReleaseChange::Mask) {
                // Inspect the argument delivered by Base, not a builder's
                // independent state read: both the handler and policy are fresh.
                assert_eq!(delivered.len(), 1);
                assert!(delivered[0].is_masked());
                assert!(delivered[0].is_editable());
                assert!(!delivered[0].is_copyable());
            } else {
                assert!(delivered.is_empty());
            }
        });
    }
}

#[gpui_kit::test]
fn policy_changes_escape_blur_and_deactivation_cancel_pending_press(cx: &mut TestAppContext) {
    let (handle, view) = fields(cx);
    for mode in 0..4 {
        cx.update_window(handle.into(), |_, window, cx| {
            let pad = padding(window, &view, cx);
            right(window, pad, true, cx);
            let input = view.read(cx).input.clone();
            match mode {
                0 => input.update(cx, |state, _| state.set_context_menu_enabled(false)),
                1 => input.update(cx, |state, cx| state.set_disabled(true, cx)),
                2 => window.press("escape", cx),
                _ => window.blur(cx),
            }
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            let pad = padding(window, &view, cx);
            right(window, pad, false, cx);
            let input = view.read(cx).input.clone();
            input.update(cx, |state, cx| {
                state.set_context_menu_enabled(true);
                state.set_disabled(false, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
    }
    // A read-only Escape can be consumed by an ancestor action binding.
    common::update_content(handle, &view, cx, |view, _, cx| {
        view.readonly = true;
        cx.notify();
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let pad = padding(window, &view, cx);
        right(window, pad, true, cx);
        window.press("escape", cx);
        right(window, pad, false, cx);
        right(window, pad, true, cx);
    })
    .unwrap();
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.deactivate_window();
        visual.run_until_parked();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.activate_window();
        let pad = padding(window, &view, cx);
        right(window, pad, false, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(request_count(&view, cx), 0);
}

#[gpui_kit::test]
fn readonly_and_masked_padding_requests_keep_clipboard_protections(cx: &mut TestAppContext) {
    let (handle, view) = fields(cx);
    common::update_content(handle, &view, cx, |view, _, cx| {
        view.readonly = true;
        cx.notify();
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("field", cx);
        window.press(SELECT_ALL, cx);
        let pad = padding(window, &view, cx);
        right(window, pad, true, cx);
        right(window, pad, false, cx);
        window.dispatch_action(Box::new(Copy), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let requests = view.read(cx).requests.borrow();
        let request = requests.last().unwrap();
        assert!(request.capabilities.is_readonly());
        assert!(request.capabilities.is_copyable());
        assert!(!request.capabilities.is_editable());
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("abcdef")
        );
        drop(requests);
        view.update(cx, |view, cx| {
            view.readonly = false;
            cx.notify();
        });
        let input = view.read(cx).input.clone();
        input.update(cx, |state, cx| state.set_masked(true, window, cx));
        cx.write_to_clipboard(ClipboardItem::new_string("canary".into()));
        window.render_frame(cx);
        let pad = padding(window, &view, cx);
        right(window, pad, true, cx);
        right(window, pad, false, cx);
        window.dispatch_action(Box::new(Copy), cx);
        window.dispatch_action(Box::new(Cut), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let requests = view.read(cx).requests.borrow();
        assert_eq!(requests.len(), 2);
        let capabilities = requests.last().unwrap().capabilities;
        assert!(capabilities.is_editable());
        assert!(capabilities.is_masked());
        assert!(!capabilities.is_copyable());
        assert_eq!(window.find("field").value(), None);
        assert_eq!(view.read(cx).input.read(cx).value(), "abcdef");
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("canary")
        );
    })
    .unwrap();
}

// Exercise the actual default presenter where a headless menu is available.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[gpui_kit::test]
fn default_padding_menu_pastes_through_the_real_drawn_menu(cx: &mut TestAppContext) {
    let (handle, view) = fields(cx);
    common::update_content(handle, &view, cx, |view, _, cx| {
        view.recording = false;
        cx.notify();
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("field", cx);
        window.press(SELECT_ALL, cx);
        cx.write_to_clipboard(ClipboardItem::new_string("pasted".into()));
        let pad = padding(window, &view, cx);
        right(window, pad, true, cx);
        right(window, pad, false, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let mut menu = window.within("popup-menu");
        assert_eq!(menu.find(2usize).label(), Some("Paste"));
        menu.click(2usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("field").value(), Some("pasted"));
        assert_eq!(window.find("other").value(), Some("unrelated"));
    })
    .unwrap();
}
