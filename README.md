# Falcão

Motor JavaScript escrito do zero em Rust, sem dependências, para a suíte FeatherOffice:
roda as macros do FeatherSheet e o JavaScript das páginas do FeatherWeb.

- `falcao-sintaxe`: léxico e árvore sintática.
- `falcao-bytecode`: as instruções e o gerador.
- `falcao-runtime`: valores, gaiola (heap isolado), formas, coletor de lixo e o intérprete.

O desenho está em [`ARQUITETURA.md`](ARQUITETURA.md).

```bash
cargo test --release
cargo run --release -p falcao-runtime --example rodar -- arquivo.js
```

Licença MIT.
