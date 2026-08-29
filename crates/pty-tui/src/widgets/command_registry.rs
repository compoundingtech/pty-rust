//! Command registry (`src/tui/widgets/command-registry.ts`): commands
//! contributed from anywhere, grouped by scope. `register_global` appends
//! to the global scope; `use_scope(id, cmds)` replaces that scope's batch;
//! disposers remove what they registered. `all()` flattens every scope in
//! insertion order (the global scope first when it was registered first)
//! and `rev()` bumps on every change so a host can memoise.

use std::cell::RefCell;
use std::rc::Rc;

use super::command_palette::Command;

const GLOBAL: &str = "__global__";

struct Inner {
    rev: u64,
    /// Insertion-ordered scopes.
    scopes: Vec<(String, Vec<Command>)>,
}

/// The registry; clones share it.
#[derive(Clone)]
pub struct CommandRegistry {
    inner: Rc<RefCell<Inner>>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Removes what a registration added; idempotent.
pub struct CommandDisposer {
    registry: CommandRegistry,
    scope: String,
    /// `Some(id)` for a single global command, `None` for a whole scope.
    id: Option<String>,
    done: bool,
}

impl CommandDisposer {
    pub fn dispose(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        match &self.id {
            Some(id) => self.registry.remove_global(id),
            None => self.registry.clear_scope(&self.scope),
        }
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        CommandRegistry {
            inner: Rc::new(RefCell::new(Inner {
                rev: 0,
                scopes: Vec::new(),
            })),
        }
    }

    fn touch(inner: &mut Inner) {
        inner.rev += 1;
    }

    /// Bumps on every change.
    pub fn rev(&self) -> u64 {
        self.inner.borrow().rev
    }

    /// `registerGlobalCommand`.
    pub fn register_global(&self, cmd: Command) -> CommandDisposer {
        let id = cmd.id.clone();
        {
            let mut inner = self.inner.borrow_mut();
            match inner.scopes.iter_mut().find(|(s, _)| s == GLOBAL) {
                Some((_, list)) => list.push(cmd),
                None => inner.scopes.push((GLOBAL.to_string(), vec![cmd])),
            }
            Self::touch(&mut inner);
        }
        CommandDisposer {
            registry: self.clone(),
            scope: GLOBAL.to_string(),
            id: Some(id),
            done: false,
        }
    }

    fn remove_global(&self, id: &str) {
        let mut inner = self.inner.borrow_mut();
        let Some(pos) = inner.scopes.iter().position(|(s, _)| s == GLOBAL) else {
            return;
        };
        inner.scopes[pos].1.retain(|c| c.id != id);
        if inner.scopes[pos].1.is_empty() {
            inner.scopes.remove(pos);
        }
        Self::touch(&mut inner);
    }

    /// `useCommandScope`: replace the batch under `scope_id` (an empty
    /// batch removes the scope). Panics on the reserved `__global__` id.
    pub fn use_scope(&self, scope_id: &str, commands: Vec<Command>) -> CommandDisposer {
        assert!(scope_id != GLOBAL, "scope id \"{GLOBAL}\" is reserved");
        {
            let mut inner = self.inner.borrow_mut();
            let pos = inner.scopes.iter().position(|(s, _)| s == scope_id);
            match (pos, commands.is_empty()) {
                (Some(p), true) => {
                    inner.scopes.remove(p);
                }
                (Some(p), false) => inner.scopes[p].1 = commands,
                (None, true) => {}
                (None, false) => inner.scopes.push((scope_id.to_string(), commands)),
            }
            Self::touch(&mut inner);
        }
        CommandDisposer {
            registry: self.clone(),
            scope: scope_id.to_string(),
            id: None,
            done: false,
        }
    }

    /// `clearCommandScope` (no-op for an unknown scope).
    pub fn clear_scope(&self, scope_id: &str) {
        let mut inner = self.inner.borrow_mut();
        if let Some(p) = inner.scopes.iter().position(|(s, _)| s == scope_id) {
            inner.scopes.remove(p);
            Self::touch(&mut inner);
        }
    }

    /// `findCommand`: the first command with this id across scopes.
    pub fn find(&self, id: &str) -> Option<Command> {
        self.inner
            .borrow()
            .scopes
            .iter()
            .flat_map(|(_, l)| l.iter())
            .find(|c| c.id == id)
            .cloned()
    }

    /// `allCommands`: every command, scopes in insertion order.
    pub fn all(&self) -> Vec<Command> {
        self.inner
            .borrow()
            .scopes
            .iter()
            .flat_map(|(_, l)| l.iter().cloned())
            .collect()
    }

    /// `runCommand`: run by id; false when unknown.
    pub fn run(&self, id: &str) -> bool {
        match self.find(id) {
            Some(cmd) => {
                cmd.run();
                true
            }
            None => false,
        }
    }

    /// `_resetCommandRegistry`.
    pub fn reset(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.scopes.clear();
        Self::touch(&mut inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn c(id: &str) -> Command {
        Command::new(id, id.to_uppercase(), || {})
    }

    fn ids(r: &CommandRegistry) -> Vec<String> {
        r.all().into_iter().map(|c| c.id).collect()
    }

    /// node: tests/widgets-command-registry.test.ts:9-28
    #[test]
    fn global() {
        let r = CommandRegistry::new();
        assert!(r.all().is_empty());
        let mut d = r.register_global(c("a"));
        assert_eq!(ids(&r), vec!["a"]);
        d.dispose();
        assert!(r.all().is_empty());
        r.register_global(c("a"));
        r.register_global(c("b"));
        assert_eq!(ids(&r), vec!["a", "b"]);
    }

    /// node: tests/widgets-command-registry.test.ts:30-77
    #[test]
    fn scopes() {
        let r = CommandRegistry::new();
        r.use_scope("s1", vec![c("s1.new"), c("s1.remove")]);
        assert_eq!(ids(&r), vec!["s1.new", "s1.remove"]);
        r.reset();
        r.use_scope("sel", vec![c("a")]);
        r.use_scope("sel", vec![c("b")]);
        assert_eq!(ids(&r), vec!["b"]);
        r.reset();
        let mut d = r.use_scope("sel", vec![c("a"), c("b")]);
        d.dispose();
        assert!(r.all().is_empty());
        r.use_scope("sel", vec![c("a")]);
        r.clear_scope("sel");
        assert!(r.all().is_empty());
        assert!(std::panic::catch_unwind(|| CommandRegistry::new().use_scope("__global__", vec![])).is_err());
        r.register_global(c("g.quit"));
        r.use_scope("screen:list", vec![c("list.new")]);
        r.use_scope("focused:a", vec![c("a.complete")]);
        let all = ids(&r);
        assert!(all.contains(&"g.quit".to_string()));
        assert!(all.contains(&"list.new".to_string()));
        assert!(all.contains(&"a.complete".to_string()));
        assert_eq!(all.len(), 3);
    }

    /// node: tests/widgets-command-registry.test.ts:79-96
    #[test]
    fn lookup_and_run() {
        let r = CommandRegistry::new();
        r.use_scope("s", vec![c("x")]);
        assert_eq!(r.find("x").map(|c| c.id).as_deref(), Some("x"));
        assert!(r.find("nope").is_none());
        let called = Rc::new(Cell::new(0));
        let cc = called.clone();
        r.use_scope("s", vec![Command::new("x", "X", move || cc.set(cc.get() + 1))]);
        assert!(r.run("x"));
        assert_eq!(called.get(), 1);
        assert!(!r.run("nope"));
    }

    /// node: tests/widgets-command-registry.test.ts:98-106
    #[test]
    fn rev_bumps_on_change() {
        let r = CommandRegistry::new();
        let r0 = r.rev();
        let mut d = r.register_global(c("q"));
        assert_eq!(r.all().len(), 1);
        assert!(r.rev() > r0);
        r.use_scope("s", vec![c("a")]);
        assert_eq!(r.all().len(), 2);
        d.dispose();
        assert_eq!(r.all().len(), 1);
    }
}
