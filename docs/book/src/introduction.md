# The Aperio Programming Language

> **Aperio** /ah-PEH-ree-oh/ — Latin: *I open. I reveal.*
>
> A spell cast at compile time; running it opens a **lotus** — a structure
> that grows itself, audits its own correctness, and dissolves cleanly when
> its work is done.

This book is a layered tutorial. Each chapter assumes the previous one and
introduces the next primitive in the substrate-up stack. By the end you'll
have written a multi-binary cross-process program with audited closures,
typed bus dispatch, generic types, and clean failure cascades — all from
first principles, no surprise machinery.

## What this book is

A *tutorial*. Read it cover-to-cover.

If you want exhaustive answers (the grammar of every construct, the precise
semantics of every operator, the full memory model), reach for the
[Aperio Reference](../../reference/book/index.html) instead.

If you want recipes ("how do I X?") — those land in the [stdlib
reference](../../std/book/index.html) once Phase 1+ libraries ship.

## Prerequisites

You'll be more comfortable if you've used a compiled, statically-typed
systems language before (Rust, Go, C, Swift, Zig). Familiarity with actor
systems, message-passing, or functional reactive programming helps but isn't
required.

You don't need to know anything about Aperio's substrate philosophy yet —
[chapter 1](./01-why-aperio.md) is the framing.

## The Aperio / lotus split

This is load-bearing terminology in this book and worth getting right
upfront:

- **Aperio** — the language. The thing you write source code in. The
  toolchain (`aperio build`, `aperio run`).
- **a lotus** — the runtime data structure an Aperio program *is*. A tree
  of *loci* (singular: *locus*) communicating via vertical-only-flow over a
  shared bus. Lowercase. "Your Aperio program opens a lotus."
- **the Lotus** (capitalized, only when grammar demands it) — the universal
  pattern, the conceptual artifact this whole framework is built around.
- **a locus** — the unit of structure inside a lotus. Plural: **loci**.
  (Latin keeps Latin.)

When this book says "the lotus" it means *the running data structure of
the program you're reading about*. When it says "Aperio" it means *the
language you're writing source code in*. Keeping these straight makes
several later chapters click.
