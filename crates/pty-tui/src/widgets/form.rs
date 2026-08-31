//! Form (`src/tui/widgets/form.ts:157-256`): a focus ring over named text
//! fields. Keys: `tab` / `backtab` walk the ring (wrapping), `return` is
//! `Activate` in a non-last field and `Submit` in the last, `escape` is
//! `Cancel`, anything else edits the focused field through
//! [`crate::line_edit::apply_text_key`]. Field rendering is
//! [`crate::line_edit::render_field_spans`].

use crate::input::KeyEvent;
use crate::line_edit::{TextFieldState, apply_text_key};

/// `FormState<Id>` (`form.ts:160-164`): fields in `order`, the focused id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormState {
    pub order: Vec<String>,
    pub values: Vec<TextFieldState>,
    /// `None` only when there are no fields.
    pub focused: Option<String>,
}

/// What a key did (`HandleFormKeyResult.action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    Edited,
    Moved,
    Submit,
    Cancel,
    Activate,
    None,
}

impl FormState {
    /// `createFormState`: `initial` values by position, cursor at the end,
    /// the first field focused.
    pub fn new(order: &[&str], initial: &[&str]) -> Self {
        FormState {
            order: order.iter().map(|s| s.to_string()).collect(),
            values: order
                .iter()
                .enumerate()
                .map(|(i, _)| TextFieldState::new(initial.get(i).copied().unwrap_or("")))
                .collect(),
            focused: order.first().map(|s| s.to_string()),
        }
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.order.iter().position(|o| o == id)
    }

    /// The field with this id.
    pub fn field(&self, id: &str) -> Option<&TextFieldState> {
        self.index_of(id).map(|i| &self.values[i])
    }

    /// The field text.
    pub fn text(&self, id: &str) -> &str {
        self.field(id).map(|f| f.text.as_str()).unwrap_or("")
    }

    fn walk(&self, delta: i64) -> Self {
        let Some(f) = &self.focused else {
            return self.clone();
        };
        let Some(idx) = self.index_of(f) else {
            return self.clone();
        };
        let n = self.order.len() as i64;
        let next = ((idx as i64 + delta) % n + n) % n;
        FormState {
            focused: Some(self.order[next as usize].clone()),
            ..self.clone()
        }
    }

    /// `focusField`.
    pub fn focus(&self, id: &str) -> Self {
        if self.index_of(id).is_none() {
            return self.clone();
        }
        FormState {
            focused: Some(id.to_string()),
            ..self.clone()
        }
    }

    /// `setFieldText`: replace a field's text, cursor at the end.
    pub fn set_text(&self, id: &str, text: &str) -> Self {
        let Some(i) = self.index_of(id) else {
            return self.clone();
        };
        let mut values = self.values.clone();
        values[i] = TextFieldState::new(text);
        FormState {
            values,
            ..self.clone()
        }
    }
}

/// `handleFormKey` (`form.ts:225-256`).
pub fn handle_form_key(state: &FormState, key: &KeyEvent) -> (FormState, FormAction) {
    match key.name.as_str() {
        "tab" => return (state.walk(1), FormAction::Moved),
        "backtab" => return (state.walk(-1), FormAction::Moved),
        "escape" => return (state.clone(), FormAction::Cancel),
        "return" => {
            let Some(f) = &state.focused else {
                return (state.clone(), FormAction::None);
            };
            let idx = state.index_of(f).unwrap_or(0);
            let is_last = idx + 1 == state.order.len();
            return (
                state.clone(),
                if is_last { FormAction::Submit } else { FormAction::Activate },
            );
        }
        _ => {}
    }
    let Some(f) = &state.focused else {
        return (state.clone(), FormAction::None);
    };
    let Some(idx) = state.index_of(f) else {
        return (state.clone(), FormAction::None);
    };
    match apply_text_key(&state.values[idx], key) {
        Some(updated) => {
            let mut values = state.values.clone();
            values[idx] = updated;
            (
                FormState {
                    values,
                    ..state.clone()
                },
                FormAction::Edited,
            )
        }
        None => (state.clone(), FormAction::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: [&str; 3] = ["title", "notes", "due"];

    /// node: tests/widgets-form.test.ts:179-219
    #[test]
    fn focus_ring() {
        let s0 = FormState::new(&ORDER, &["", "", ""]);
        let (s1, a) = handle_form_key(&s0, &KeyEvent::named("tab"));
        assert_eq!((s1.focused.as_deref(), a), (Some("notes"), FormAction::Moved));
        let (s2, _) = handle_form_key(&s1, &KeyEvent::named("tab"));
        assert_eq!(s2.focused.as_deref(), Some("due"));
        let (s3, _) = handle_form_key(&s2, &KeyEvent::named("tab"));
        assert_eq!(s3.focused.as_deref(), Some("title"));
        let (s4, _) = handle_form_key(&s3, &KeyEvent::named("backtab"));
        assert_eq!(s4.focused.as_deref(), Some("due"));
        assert_eq!(handle_form_key(&s0, &KeyEvent::named("return")).1, FormAction::Activate);
        assert_eq!(handle_form_key(&s0.focus("due"), &KeyEvent::named("return")).1, FormAction::Submit);
        assert_eq!(handle_form_key(&s0, &KeyEvent::named("escape")).1, FormAction::Cancel);
        let (s, a) = handle_form_key(&s0, &KeyEvent::printable("a"));
        assert_eq!(a, FormAction::Edited);
        assert_eq!(s.text("title"), "a");
        let s0 = FormState::new(&ORDER, &["old", "", ""]);
        let s1 = s0.set_text("notes", "hello");
        assert_eq!(s1.field("notes"), Some(&TextFieldState { text: "hello".into(), cursor: 5 }));
        assert_eq!(s1.focused.as_deref(), Some("title"));
    }
}
