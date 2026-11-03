# {{name}}

An [Oxide](https://github.com/oxide-hq/oxide-framework) application: Laravel ergonomics,
compile-time safe Rust.

## Getting started

```bash
cp .env.example .env
cargo test
cargo run
```
{{database_notes}}
Then open <http://127.0.0.1:3000/api/v1/health>.

## Where things live

| Path | What goes there |
| ---- | --------------- |
| `src/context.rs` | The service container (`AppContext`) and services such as `AppMailer` |
| `src/routes/api.rs` | Routes, declared with `routes!` |
| `src/http/controllers/` | Controllers with up to five actions: `index`, `store`, `show`, `update`, `destroy` |
| `src/http/requests/` | Form requests: validated, authorized request bodies |
{{data_rows}}| `tests/` | Feature tests with `TestClient` |

## Generating code

```bash
oxide make:model Comment --migration
oxide make:controller Comment --resource
oxide make:request StoreCommentRequest
```

Each command creates the file and registers it in the parent `mod.rs`.
