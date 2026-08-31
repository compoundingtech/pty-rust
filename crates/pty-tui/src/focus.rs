//! A stack-based focus router, ported from `src/tui/focus.ts:49-134`.
//!
//! Scopes are pushed innermost-last. Dispatch walks a snapshot of the stack
//! from the innermost scope outward, skips scopes whose `active` predicate
//! says no, and stops at the first handler that returns `true`. Handlers may
//! push or remove scopes during a dispatch without disturbing the pass.

use std::cell::RefCell;
use std::rc::Rc;

use crate::input::{KeyEvent, MouseEvent};

/// A key handler: `true` consumes the event.
pub type KeyHandler<C> = Box<dyn FnMut(&KeyEvent, &mut C) -> bool>;
/// A mouse handler: `true` consumes the event.
pub type MouseHandler<C> = Box<dyn FnMut(&MouseEvent, &mut C) -> bool>;
/// The `active` predicate.
pub type ActiveFn = Box<dyn Fn() -> bool>;

/// One scope (`FocusScope`, `focus.ts:53-66`). `C` is the context handed
/// to handlers (Node's `ScreenContext`).
pub struct FocusScope<C> {
    /// For debugging; not required to be unique.
    pub id: String,
    /// Whether the scope dispatches right now. Default: always.
    pub active: Option<ActiveFn>,
    pub on_key: Option<KeyHandler<C>>,
    pub on_mouse: Option<MouseHandler<C>>,
}

impl<C> FocusScope<C> {
    /// A scope with no handlers.
    pub fn new(id: impl Into<String>) -> Self {
        FocusScope {
            id: id.into(),
            active: None,
            on_key: None,
            on_mouse: None,
        }
    }

    pub fn active(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.active = Some(Box::new(f));
        self
    }

    pub fn on_key(mut self, f: impl FnMut(&KeyEvent, &mut C) -> bool + 'static) -> Self {
        self.on_key = Some(Box::new(f));
        self
    }

    pub fn on_mouse(mut self, f: impl FnMut(&MouseEvent, &mut C) -> bool + 'static) -> Self {
        self.on_mouse = Some(Box::new(f));
        self
    }

    fn is_active(&self) -> bool {
        self.active.as_ref().is_none_or(|f| f())
    }
}

struct Entry<C> {
    serial: u64,
    id: String,
    scope: Rc<RefCell<FocusScope<C>>>,
}

struct Inner<C> {
    next_serial: u64,
    entries: Vec<Entry<C>>,
}

/// The stack (`FocusManager`, `focus.ts:68-134`). Cloning shares the stack.
pub struct FocusStack<C> {
    inner: Rc<RefCell<Inner<C>>>,
}

impl<C> Clone for FocusStack<C> {
    fn clone(&self) -> Self {
        FocusStack {
            inner: self.inner.clone(),
        }
    }
}

impl<C> Default for FocusStack<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// Removes its scope when disposed (`push` returns it; `focus.ts:88-97`).
/// Disposing twice is safe. Dropping the guard does NOT remove the scope —
/// like Node's disposer, removal is explicit.
pub struct FocusGuard<C> {
    serial: u64,
    stack: FocusStack<C>,
    disposed: bool,
}

impl<C> FocusGuard<C> {
    /// Remove the scope from the stack.
    pub fn dispose(&mut self) {
        if self.disposed {
            return;
        }
        self.disposed = true;
        self.stack.remove_serial(self.serial);
    }

    /// The scope's serial (for [`FocusStack::remove_serial`]).
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

impl<C> FocusStack<C> {
    pub fn new() -> Self {
        FocusStack {
            inner: Rc::new(RefCell::new(Inner {
                next_serial: 1,
                entries: Vec::new(),
            })),
        }
    }

    /// Push a scope; the guard removes it again.
    pub fn push(&self, scope: FocusScope<C>) -> FocusGuard<C> {
        let mut inner = self.inner.borrow_mut();
        let serial = inner.next_serial;
        inner.next_serial += 1;
        inner.entries.push(Entry {
            serial,
            id: scope.id.clone(),
            scope: Rc::new(RefCell::new(scope)),
        });
        FocusGuard {
            serial,
            stack: self.clone(),
            disposed: false,
        }
    }

    /// Remove the scope with this serial (idempotent).
    pub fn remove_serial(&self, serial: u64) {
        let mut inner = self.inner.borrow_mut();
        if let Some(i) = inner.entries.iter().position(|e| e.serial == serial) {
            inner.entries.remove(i);
        }
    }

    /// Remove every scope with this id.
    pub fn remove_id(&self, id: &str) {
        let mut inner = self.inner.borrow_mut();
        inner.entries.retain(|e| e.id != id);
    }

    /// The id of the innermost active scope, or `None`.
    pub fn current(&self) -> Option<String> {
        let inner = self.inner.borrow();
        inner
            .entries
            .iter()
            .rev()
            .find(|e| e.scope.try_borrow().is_ok_and(|s| s.is_active()))
            .map(|e| e.id.clone())
    }

    /// The ids of every scope, root → innermost.
    pub fn stack(&self) -> Vec<String> {
        self.inner
            .borrow()
            .entries
            .iter()
            .map(|e| e.id.clone())
            .collect()
    }

    /// Number of scopes.
    pub fn len(&self) -> usize {
        self.inner.borrow().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn snapshot(&self) -> Vec<Rc<RefCell<FocusScope<C>>>> {
        self.inner
            .borrow()
            .entries
            .iter()
            .map(|e| e.scope.clone())
            .collect()
    }

    /// Dispatch a key innermost-first; `true` when a scope consumed it.
    pub fn dispatch_key(&self, key: &KeyEvent, ctx: &mut C) -> bool {
        for scope in self.snapshot().into_iter().rev() {
            let Ok(mut s) = scope.try_borrow_mut() else {
                continue;
            };
            if !s.is_active() {
                continue;
            }
            if let Some(h) = s.on_key.as_mut()
                && h(key, ctx)
            {
                return true;
            }
        }
        false
    }

    /// Dispatch a mouse event innermost-first.
    pub fn dispatch_mouse(&self, event: &MouseEvent, ctx: &mut C) -> bool {
        for scope in self.snapshot().into_iter().rev() {
            let Ok(mut s) = scope.try_borrow_mut() else {
                continue;
            };
            if !s.is_active() {
                continue;
            }
            if let Some(h) = s.on_mouse.as_mut()
                && h(event, ctx)
            {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{MouseAction, MouseButton};
    use std::cell::Cell;

    type Log = Rc<RefCell<Vec<&'static str>>>;

    fn k(name: &str) -> KeyEvent {
        KeyEvent::named(name)
    }

    fn me() -> MouseEvent {
        MouseEvent {
            action: MouseAction::Press,
            button: MouseButton::Left,
            x: 0,
            y: 0,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    fn logging(log: &Log, id: &'static str, consume: bool) -> FocusScope<()> {
        let log = log.clone();
        FocusScope::new(id).on_key(move |_, _| {
            log.borrow_mut().push(id);
            consume
        })
    }

    /// node: tests/focus.test.ts:22-51
    #[test]
    fn stack_semantics() {
        let f: FocusStack<()> = FocusStack::new();
        assert_eq!(f.current(), None);
        assert!(f.stack().is_empty());
        let mut d = f.push(FocusScope::new("a"));
        assert_eq!(f.stack(), vec!["a"]);
        d.dispose();
        assert!(f.stack().is_empty());
        d.dispose();
        d.dispose();
        assert!(f.stack().is_empty());
        let _a = f.push(FocusScope::new("a"));
        let _b = f.push(FocusScope::new("b"));
        let _c = f.push(FocusScope::new("c"));
        assert_eq!(f.current().as_deref(), Some("c"));
    }

    /// node: tests/focus.test.ts:53-97
    #[test]
    fn key_bubbling() {
        let log: Log = Rc::default();
        let f: FocusStack<()> = FocusStack::new();
        let _o = f.push(logging(&log, "outer", true));
        let _i = f.push(logging(&log, "inner", true));
        f.dispatch_key(&k("a"), &mut ());
        assert_eq!(*log.borrow(), vec!["inner"]);

        let log: Log = Rc::default();
        let f: FocusStack<()> = FocusStack::new();
        let _o = f.push(logging(&log, "outer", true));
        let _m = f.push(FocusScope::new("middle"));
        let _i = f.push(logging(&log, "inner", false));
        f.dispatch_key(&k("a"), &mut ());
        assert_eq!(*log.borrow(), vec!["inner", "outer"]);

        let f: FocusStack<()> = FocusStack::new();
        let _a = f.push(FocusScope::new("a").on_key(|_, _| false));
        let _b = f.push(FocusScope::new("b").on_key(|_, _| false));
        assert!(!f.dispatch_key(&k("x"), &mut ()));
        let _c = f.push(FocusScope::new("c").on_key(|_, _| true));
        assert!(f.dispatch_key(&k("x"), &mut ()));
    }

    /// node: tests/focus.test.ts:99-133
    #[test]
    fn active_predicate() {
        let pane_is_a = Rc::new(Cell::new(true));
        let log: Log = Rc::default();
        let f: FocusStack<()> = FocusStack::new();
        let (p, l) = (pane_is_a.clone(), log.clone());
        let _a = f.push(
            FocusScope::new("A")
                .active(move || p.get())
                .on_key(move |_, _| {
                    l.borrow_mut().push("A");
                    true
                }),
        );
        let (p, l) = (pane_is_a.clone(), log.clone());
        let _b = f.push(
            FocusScope::new("B")
                .active(move || !p.get())
                .on_key(move |_, _| {
                    l.borrow_mut().push("B");
                    true
                }),
        );
        f.dispatch_key(&k("x"), &mut ());
        assert_eq!(*log.borrow(), vec!["A"]);
        log.borrow_mut().clear();
        pane_is_a.set(false);
        f.dispatch_key(&k("x"), &mut ());
        assert_eq!(*log.borrow(), vec!["B"]);

        let f: FocusStack<()> = FocusStack::new();
        let _a = f.push(FocusScope::new("A").active(|| false));
        let _b = f.push(FocusScope::new("B").active(|| true));
        assert_eq!(f.stack(), vec!["A", "B"]);
        assert_eq!(f.current().as_deref(), Some("B"));
    }

    /// node: tests/focus.test.ts:135-196
    #[test]
    fn nested_app_shape_and_pop_mid_dispatch() {
        let log: Log = Rc::default();
        let f: FocusStack<()> = FocusStack::new();
        let l = log.clone();
        let _g = f.push(FocusScope::new("global").on_key(move |key, _| {
            l.borrow_mut().push("global");
            key.name == "c" && key.ctrl
        }));
        let l = log.clone();
        let _p = f.push(FocusScope::new("pane").on_key(move |key, _| {
            l.borrow_mut().push("pane");
            key.ch.as_deref() == Some("n")
        }));
        let l = log.clone();
        let mut modal = f.push(FocusScope::new("modal").on_key(move |key, _| {
            l.borrow_mut().push("modal");
            key.name == "escape"
        }));
        f.dispatch_key(&k("escape"), &mut ());
        assert_eq!(*log.borrow(), vec!["modal"]);
        log.borrow_mut().clear();
        f.dispatch_key(&KeyEvent::printable("n"), &mut ());
        assert_eq!(*log.borrow(), vec!["modal", "pane"]);
        log.borrow_mut().clear();
        f.dispatch_key(&KeyEvent::ctrl("c"), &mut ());
        assert_eq!(*log.borrow(), vec!["modal", "pane", "global"]);
        modal.dispose();
        log.borrow_mut().clear();
        f.dispatch_key(&k("escape"), &mut ());
        assert_eq!(*log.borrow(), vec!["pane", "global"]);

        // A handler that removes its own scope mid-dispatch.
        let log: Log = Rc::default();
        let f: FocusStack<()> = FocusStack::new();
        let _o = f.push(logging(&log, "outer", true));
        let stack = f.clone();
        let inner = f.push(FocusScope::new("inner"));
        let serial = inner.serial();
        f.remove_serial(serial);
        let _i = f.push(FocusScope::new("inner").on_key(move |_, _| {
            stack.remove_id("inner");
            false
        }));
        f.dispatch_key(&k("x"), &mut ());
        assert_eq!(*log.borrow(), vec!["outer"]);
        assert_eq!(f.stack(), vec!["outer"]);
    }

    /// node: tests/focus.test.ts:198-213
    #[test]
    fn mouse_dispatch() {
        let log: Log = Rc::default();
        let f: FocusStack<()> = FocusStack::new();
        let l = log.clone();
        let _o = f.push(FocusScope::new("o").on_mouse(move |_, _| {
            l.borrow_mut().push("o");
            false
        }));
        let l = log.clone();
        let _i = f.push(FocusScope::new("i").on_mouse(move |_, _| {
            l.borrow_mut().push("i");
            false
        }));
        f.dispatch_mouse(&me(), &mut ());
        assert_eq!(*log.borrow(), vec!["i", "o"]);

        let keys = Rc::new(Cell::new(0));
        let mice = Rc::new(Cell::new(0));
        let f: FocusStack<()> = FocusStack::new();
        let kc = keys.clone();
        let _a = f.push(FocusScope::new("a").on_key(move |_, _| {
            kc.set(kc.get() + 1);
            true
        }));
        let mc = mice.clone();
        let _b = f.push(FocusScope::new("b").on_mouse(move |_, _| {
            mc.set(mc.get() + 1);
            true
        }));
        f.dispatch_key(&k("x"), &mut ());
        f.dispatch_mouse(&me(), &mut ());
        assert_eq!((keys.get(), mice.get()), (1, 1));
    }
}
