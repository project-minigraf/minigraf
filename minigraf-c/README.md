# minigraf C bindings

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database — C header + prebuilt shared library

Minigraf for C and any language with a C FFI. Distributed as platform tarballs containing `minigraf.h` and prebuilt shared and static libraries.

## Install

Download the platform tarball from [GitHub Releases](https://github.com/project-minigraf/minigraf/releases):

```sh
# Linux x86_64 example
curl -L https://github.com/project-minigraf/minigraf/releases/download/v1.0.0/minigraf-c-v1.0.0-x86_64-unknown-linux-gnu.tar.gz | tar xz
```

Each archive contains:
- `include/minigraf.h` — stable C header (cbindgen-generated)
- `lib/libminigraf_c.so` (Linux) / `libminigraf_c.dylib` (macOS) / `minigraf_c.dll` (Windows)
- `lib/libminigraf_c.a` (Linux/macOS) / `minigraf_c.lib` (Windows)

## Quick start

```c
#include "minigraf.h"
#include <stdio.h>
#include <stdlib.h>

int main(void) {
    char *err = NULL;
    MiniGrafDb *db = minigraf_open("myapp.graph", &err);
    if (!db) { fprintf(stderr, "open: %s\n", err); free(err); return 1; }

    char *result = minigraf_execute(
        db,
        "(transact [[:alice :person/name \"Alice\"] [:alice :person/age 30]])",
        &err
    );
    if (!result) { fprintf(stderr, "execute: %s\n", err); free(err); }
    else { printf("%s\n", result); minigraf_string_free(result); }

    result = minigraf_execute(
        db,
        "(query [:find ?name :where [?e :person/name ?name]])",
        &err
    );
    if (!result) { fprintf(stderr, "execute: %s\n", err); free(err); }
    else { printf("%s\n", result); minigraf_string_free(result); }

    minigraf_checkpoint(db, NULL);
    minigraf_close(db);
    return 0;
}
```

## Memory contract

Mirrors SQLite:
- `minigraf_open` — caller owns the `MiniGrafDb*`; free with `minigraf_close`
- `minigraf_execute` — returns a heap-allocated JSON string; caller must free with `minigraf_string_free`
- Error strings (`char **err` out-param) are heap-allocated; caller frees with `free()`

## API summary

| Function | Description |
|---|---|
| `minigraf_open(path, err)` | Open or create a file-backed database |
| `minigraf_open_in_memory(err)` | Open an in-memory database |
| `minigraf_execute(db, datalog, err)` | Execute a Datalog command; returns JSON string |
| `minigraf_string_free(s)` | Free a string returned by `minigraf_execute` |
| `minigraf_checkpoint(db, err)` | Flush dirty pages to disk |
| `minigraf_close(db)` | Close the database |
| `minigraf_last_error()` | Get the last error message (thread-local) |

See `include/minigraf.h` for the complete API.

## Links

- [Full C FFI integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#c-ffi)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
