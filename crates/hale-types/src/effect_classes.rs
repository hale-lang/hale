//! The user effect-class table (#345, #354; F.40 phase 3).
//!
//! A load parses every seed through one class table
//! (`hale_syntax::ast::EffectClasses`), so an `EffectClass::User(i)`
//! names one class in every seed: the class's name is its identity, and
//! the index is assigned once, at the declaration layer, never
//! renumbered by a merge. [`EffectClassTable`] is that table as every
//! analysis reads it — the names, which were declared, which are
//! composed — and it owns the one expansion of a composed class
//! (`effect io = { syscall, block };`): its mask, its atoms, and whether
//! its definition is cyclic.

use std::collections::BTreeSet;

use hale_syntax::ast::{EffectClass, Program};

use crate::stdlib_surface::EffectSet;

/// The bundle's user effect classes.
#[derive(Debug, Clone, Default)]
pub struct EffectClassTable {
    names: Vec<String>,
    declared: BTreeSet<u16>,
    defs: Vec<Option<Vec<EffectClass>>>,
}

impl EffectClassTable {
    /// The table of a bundle's programs. A load's programs carry the
    /// load's one table as each parse left it, every one a prefix of the
    /// last: the names are the longest, a definition is the one any
    /// program records, and a class is declared when any program
    /// declared it.
    pub fn of(programs: &[&Program]) -> EffectClassTable {
        let Some(longest) = programs.iter().copied().reduce(|a, b| {
            if b.effect_names.len() > a.effect_names.len() { b } else { a }
        }) else {
            return EffectClassTable::default();
        };
        let mut defs = longest.effect_defs.clone();
        defs.resize(longest.effect_names.len(), None);
        for p in programs {
            for (i, d) in p.effect_defs.iter().enumerate() {
                if let (Some(slot @ None), Some(_)) = (defs.get_mut(i), d) {
                    *slot = d.clone();
                }
            }
        }
        EffectClassTable {
            names: longest.effect_names.clone(),
            declared: programs.iter().flat_map(|p| p.declared_effects.iter().copied()).collect(),
            defs,
        }
    }

    /// Every class name, indexed by `EffectClass::User`.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The indices an `effect NAME;` declaration introduced. A name only
    /// referenced (a typo in an `@effects(...)` clause) is interned but
    /// not declared.
    pub fn declared(&self) -> &BTreeSet<u16> {
        &self.declared
    }

    /// A reference to class `i` no declaration introduced: the name as
    /// written, and the did-you-mean hint naming the nearest declared
    /// class one typing slip away (empty when there is none).
    pub fn undeclared(&self, i: u16) -> (String, String) {
        let bad = self.names.get(i as usize).cloned().unwrap_or_default();
        let mut near: Vec<&String> = self
            .names
            .iter()
            .enumerate()
            .filter(|(j, _)| self.declared.contains(&(*j as u16)))
            .map(|(_, n)| n)
            .filter(|n| crate::effects::close(n, &bad))
            .collect();
        near.sort();
        let hint = match near.first() {
            Some(n) => format!(" Did you mean `{}`?", n),
            None => String::new(),
        };
        (bad, hint)
    }

    /// Whether class `i` is defined as a union of others.
    pub fn is_composed(&self, i: u16) -> bool {
        matches!(self.defs.get(i as usize), Some(Some(_)))
    }

    /// The name the author wrote for `class`: a user class's from the
    /// table, a built-in's own.
    pub fn display(&self, class: EffectClass) -> String {
        match class {
            EffectClass::User(i) => self
                .names
                .get(i as usize)
                .cloned()
                .unwrap_or_else(|| "<user effect>".to_string()),
            _ => class.as_str().to_string(),
        }
    }

    /// The one expansion: every atom `class` stands for (a built-in, or
    /// a user class with no definition), each passed to `atom`, following
    /// definitions depth-first. A definition re-entered on the current
    /// path (a cycle, diagnosed at the declaration) contributes nothing
    /// there; the return says whether that happened.
    fn expand(&self, class: EffectClass, atom: &mut dyn FnMut(EffectClass), path: &mut Vec<u16>) -> bool {
        let EffectClass::User(i) = class else {
            atom(class);
            return false;
        };
        let Some(Some(members)) = self.defs.get(i as usize) else {
            atom(class);
            return false;
        };
        if path.contains(&i) {
            return true;
        }
        path.push(i);
        let mut cyclic = false;
        for m in members {
            cyclic |= self.expand(*m, atom, path);
        }
        path.pop();
        cyclic
    }

    /// `class`'s mask: a composed class owns no bit, so its mask is the
    /// union of its members' (#354). Forbidding `io` then tests against
    /// `syscall|block`, and a fn that reaches a syscall carries `io`.
    pub fn mask(&self, class: EffectClass) -> EffectSet {
        let mut acc = EffectSet::PURE;
        self.expand(class, &mut |a| acc = acc.union(crate::frontier::class_mask(a)), &mut Vec::new());
        acc
    }

    /// The atoms class `i` normalizes to, by name, and whether its
    /// expansion re-entered a class (a cycle, which resolves to no
    /// effect and must stay distinguishable from an atomic class).
    pub fn atoms(&self, i: u16) -> (BTreeSet<String>, bool) {
        let mut out = BTreeSet::new();
        let cyclic = self.expand(
            EffectClass::User(i),
            &mut |a| {
                let name = match a {
                    EffectClass::User(j) => self.names.get(j as usize).cloned(),
                    b => Some(b.as_str().to_string()),
                };
                out.extend(name);
            },
            &mut Vec::new(),
        );
        (out, cyclic)
    }

    /// Whether class `i` is defined in terms of itself: its definition
    /// reaches it again. Such a class resolves to no effect, so a
    /// contract naming it would hold vacuously; it is refused.
    pub fn defined_in_terms_of_itself(&self, i: u16) -> bool {
        let mut seen: Vec<u16> = vec![i];
        let mut queue: Vec<u16> = self.user_members(i).collect();
        while let Some(j) = queue.pop() {
            if j == i {
                return true;
            }
            if seen.contains(&j) {
                continue;
            }
            seen.push(j);
            queue.extend(self.user_members(j));
        }
        false
    }

    fn user_members(&self, i: u16) -> impl Iterator<Item = u16> + '_ {
        self.defs
            .get(i as usize)
            .and_then(|d| d.as_deref())
            .unwrap_or(&[])
            .iter()
            .filter_map(|m| match m {
                EffectClass::User(j) => Some(*j),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::ast::EffectClasses;

    /// Two files parsed through one load table share one numbering, and
    /// the table expands a composed class once: its mask, its atoms, and
    /// a cyclic definition refused.
    #[test]
    fn one_table_one_expansion() {
        let mut load = EffectClasses::default();
        let a = hale_syntax::parse_source_at_in(
            "effect money;\neffect io = { syscall, money };\nfn main() { }\n",
            0,
            &mut load,
        )
        .expect("a parses");
        let b = hale_syntax::parse_source_at_in(
            "effect pii;\neffect loop_a = { loop_b };\neffect loop_b = { loop_a };\n\
             @effects(none: { money })\nfn f() { }\n",
            1000,
            &mut load,
        )
        .expect("b parses");
        let t = EffectClassTable::of(&[&a, &b]);
        // a definition's members are interned before the class it defines
        assert_eq!(t.names(), ["money", "io", "pii", "loop_b", "loop_a"]);
        assert_eq!(t.declared().iter().copied().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        let money = t.mask(EffectClass::User(0));
        assert_eq!(t.mask(EffectClass::User(1)), EffectSet::SYSCALL.union(money));
        let atoms: BTreeSet<String> = ["money", "syscall"].iter().map(|s| s.to_string()).collect();
        assert_eq!(t.atoms(1), (atoms, false));
        assert!(!t.is_composed(0) && t.is_composed(1));
        assert!(t.defined_in_terms_of_itself(3) && !t.defined_in_terms_of_itself(1));
        assert!(t.atoms(3).1, "a cyclic expansion says so");
        assert_eq!(t.mask(EffectClass::User(3)), EffectSet::PURE);
        assert_eq!(t.display(EffectClass::User(2)), "pii");
        assert_eq!(t.display(EffectClass::Syscall), "syscall");
    }
}
