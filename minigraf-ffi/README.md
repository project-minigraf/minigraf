# minigraf-ffi

UniFFI bridge crate for [Minigraf](https://github.com/project-minigraf/minigraf) — a zero-config, single-file, embedded graph database with bi-temporal Datalog queries.

## Purpose

This crate is the UniFFI bridge that language binding repos use to generate their native bindings. It is not intended to be used directly.

## Language bindings

Use the language-specific repo for your platform:

| Language | Package | Repo |
|---|---|---|
| Python | [`minigraf` on PyPI](https://pypi.org/p/minigraf) | [minigraf-python](https://github.com/project-minigraf/minigraf-python) |
| Node.js | [`minigraf` on npm](https://www.npmjs.com/package/minigraf) | [minigraf-node](https://github.com/project-minigraf/minigraf-node) |
| Browser WASM | [`@minigraf/browser` on npm](https://www.npmjs.com/package/@minigraf/browser) | [minigraf-wasm](https://github.com/project-minigraf/minigraf-wasm) |
| WASI | [`@minigraf/wasi` on npm](https://www.npmjs.com/package/@minigraf/wasi) | [minigraf-wasm](https://github.com/project-minigraf/minigraf-wasm) |

## License

MIT OR Apache-2.0
