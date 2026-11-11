//! Tab between fields. iced leaves that to the application: a text input
//! takes the focus on a click and never hands it on by itself.
//!
//! The stock `focus_next` drops the focus past the last field, and it would
//! walk into the page behind an open sheet. Here the walk wraps around, and
//! stays inside the layer on top.

use iced::advanced::widget::operation::focusable::{self, Count, Focusable};
use iced::advanced::widget::operation::{self, Operation};
use iced::advanced::widget::{Id, operate};
use iced::{Rectangle, Task};

/// The page under everything else.
pub const BASE: &str = "focus-base";
/// Sheets and dialogs over the page.
pub const LAYER: &str = "focus-layer";

struct Step<const BACK: bool> {
    count: Count,
    current: usize,
}

impl<const BACK: bool> Step<BACK> {
    fn target(&self) -> usize {
        let total = self.count.total;
        match (self.count.focused, BACK) {
            (None, false) => 0,
            (None, true) => total - 1,
            (Some(focused), false) => (focused + 1) % total,
            (Some(focused), true) => (focused + total - 1) % total,
        }
    }
}

impl<T, const BACK: bool> Operation<T> for Step<BACK> {
    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if self.current == self.target() {
            state.focus();
        } else if self.count.focused == Some(self.current) {
            state.unfocus();
        }
        self.current += 1;
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }
}

fn walk<T: Send + 'static, const BACK: bool>() -> impl Operation<T> {
    operation::then(focusable::count(), |count| Step::<BACK> { count, current: 0 })
}

fn walk_inside<T: Send + 'static>(layer: &'static str, back: bool) -> Task<T> {
    if back {
        operate(operation::scope(Id::new(layer), walk::<T, true>()))
    } else {
        operate(operation::scope(Id::new(layer), walk::<T, false>()))
    }
}

/// Moves to the next field, or the previous one with `back`.
pub fn step<T: Send + 'static>(back: bool, layered: bool) -> Task<T> {
    if layered {
        // What was focused on the page must not keep taking the keys.
        Task::batch([operate(operation::scope(Id::new(BASE), focusable::unfocus())), walk_inside(LAYER, back)])
    } else {
        walk_inside(BASE, back)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::operation::Outcome;

    struct Field(bool);

    impl Focusable for Field {
        fn is_focused(&self) -> bool {
            self.0
        }
        fn focus(&mut self) {
            self.0 = true;
        }
        fn unfocus(&mut self) {
            self.0 = false;
        }
    }

    fn press<const BACK: bool>(fields: &mut [Field]) {
        let mut op: Box<dyn Operation<()>> = Box::new(walk::<(), BACK>());
        loop {
            for field in fields.iter_mut() {
                op.focusable(None, Rectangle::default(), field);
            }
            match op.finish() {
                Outcome::Chain(next) => op = next,
                _ => break,
            }
        }
    }

    fn focused(fields: &[Field]) -> Vec<usize> {
        fields.iter().enumerate().filter(|(_, f)| f.0).map(|(i, _)| i).collect()
    }

    fn three() -> [Field; 3] {
        [Field(false), Field(false), Field(false)]
    }

    #[test]
    fn nothing_focused_starts_at_either_end() {
        let mut fields = three();
        press::<false>(&mut fields);
        assert_eq!(focused(&fields), [0]);
        let mut fields = three();
        press::<true>(&mut fields);
        assert_eq!(focused(&fields), [2]);
    }

    #[test]
    fn moves_one_field_at_a_time_in_both_directions() {
        let mut fields = three();
        press::<false>(&mut fields);
        press::<false>(&mut fields);
        assert_eq!(focused(&fields), [1]);
        press::<true>(&mut fields);
        assert_eq!(focused(&fields), [0]);
    }

    #[test]
    fn wraps_instead_of_losing_the_focus() {
        let mut fields = three();
        fields[2].0 = true;
        press::<false>(&mut fields);
        assert_eq!(focused(&fields), [0]);
        press::<true>(&mut fields);
        assert_eq!(focused(&fields), [2]);
    }

    #[test]
    fn a_single_field_keeps_the_focus() {
        let mut fields = [Field(true)];
        press::<false>(&mut fields);
        assert_eq!(focused(&fields), [0]);
    }

    #[test]
    fn no_fields_is_not_an_error() {
        press::<false>(&mut []);
        press::<true>(&mut []);
    }

    /// The ids of everything holding the focus, in a tree of real text inputs.
    struct Seen(Vec<Id>);

    impl Operation for Seen {
        fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
            if state.is_focused() {
                self.0.extend(id.cloned());
            }
        }

        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }
    }

    fn field(name: &'static str) -> iced::Element<'static, ()> {
        iced::widget::text_input("", "").id(Id::new(name)).on_input(|_| ()).into()
    }

    #[test]
    fn a_sheet_keeps_the_walk_inside_itself() {
        use iced::advanced::renderer::Headless;
        use iced::widget::{column, container, stack};
        use iced_runtime::user_interface::{Cache, UserInterface};

        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let mut renderer: iced::Renderer =
            runtime.block_on(<iced::Renderer as Headless>::new(iced::Font::default(), iced::Pixels(14.0), Some("tiny-skia"))).expect("a renderer");
        let page = container(column![field("page-a"), field("page-b")]).id(BASE);
        let sheet = container(column![field("sheet-a"), field("sheet-b")]).id(LAYER);
        let mut ui: UserInterface<'_, (), iced::Theme, iced::Renderer> =
            UserInterface::build(stack![page, sheet], iced::Size::new(400.0, 300.0), Cache::default(), &mut renderer);

        let mut run = |first: Box<dyn Operation>, renderer: &iced::Renderer| {
            let mut op = first;
            loop {
                ui.operate(renderer, op.as_mut());
                match op.finish() {
                    Outcome::Chain(next) => op = next,
                    _ => break,
                }
            }
            let mut seen = Seen(Vec::new());
            ui.operate(renderer, &mut seen);
            seen.0
        };
        let next = || -> Box<dyn Operation> { Box::new(operation::scope(Id::new(LAYER), walk::<(), false>())) };
        let previous = || -> Box<dyn Operation> { Box::new(operation::scope(Id::new(LAYER), walk::<(), true>())) };

        assert_eq!(run(Box::new(operation::scope(Id::new(BASE), walk::<(), false>())), &renderer), [Id::new("page-a")]);

        // The sheet opens over a page with a field still focused.
        assert_eq!(run(Box::new(operation::scope(Id::new(BASE), focusable::unfocus())), &renderer), []);
        assert_eq!(run(next(), &renderer), [Id::new("sheet-a")]);
        assert_eq!(run(next(), &renderer), [Id::new("sheet-b")]);
        assert_eq!(run(next(), &renderer), [Id::new("sheet-a")]);
        assert_eq!(run(previous(), &renderer), [Id::new("sheet-b")]);
    }
}
