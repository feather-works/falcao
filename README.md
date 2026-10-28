# Falcão

A JavaScript engine written from scratch in Rust, with no dependencies, for the FeatherOffice suite:
it runs FeatherSheet macros and the JavaScript of FeatherWeb pages.

- `falcao-sintaxe`: lexer and syntax tree.
- `falcao-bytecode`: the instructions and the bytecode generator.
- `falcao-runtime`: values, the cage (isolated heap), shapes, garbage collector and the interpreter.
- `falcao-asm`: x86-64 assembler used by the JIT.

The design is described in [`ARQUITETURA.md`](ARQUITETURA.md) (in Portuguese).

```bash
cargo test --release
cargo run --release -p falcao-runtime --example rodar -- file.js
```

MIT license.
