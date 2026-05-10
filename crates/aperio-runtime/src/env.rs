//! Environment for the interpreter.
//!
//! Frame-stack scoping with parent pointers via index. Each
//! `let` pushes a binding into the current frame; blocks push
//! and pop frames; functions get a fresh root frame chained to
//! the global environment.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Outermost-first stack of frames.
    frames: Vec<Rc<RefCell<Frame>>>,
}

#[derive(Debug, Default)]
struct Frame {
    bindings: BTreeMap<String, Value>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            frames: vec![Rc::new(RefCell::new(Frame::default()))],
        }
    }

    pub fn push(&mut self) {
        self.frames.push(Rc::new(RefCell::new(Frame::default())));
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn define(&self, name: impl Into<String>, value: Value) {
        self.frames
            .last()
            .expect("at least one frame")
            .borrow_mut()
            .bindings
            .insert(name.into(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<Value> {
        for frame in self.frames.iter().rev() {
            if let Some(v) = frame.borrow().bindings.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    pub fn assign(&self, name: &str, value: Value) -> bool {
        for frame in self.frames.iter().rev() {
            if frame.borrow().bindings.contains_key(name) {
                frame.borrow_mut().bindings.insert(name.to_string(), value);
                return true;
            }
        }
        false
    }
}
