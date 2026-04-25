#ifndef MINIGRAF_H
#define MINIGRAF_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdint.h>

typedef struct MiniGrafDb MiniGrafDb;

// Open a file-backed Minigraf database. Returns NULL on error.
struct MiniGrafDb *minigraf_open(const char *path);

// Open an in-memory Minigraf database. Returns NULL on error.
struct MiniGrafDb *minigraf_open_in_memory(void);

// Close a database and free all associated memory.
void minigraf_close(struct MiniGrafDb *handle);

// Execute a Datalog string. Returns a JSON string on success (caller must free
// with `minigraf_string_free`), or NULL on error (call `minigraf_last_error`).
char *minigraf_execute(struct MiniGrafDb *handle, const char *datalog);

// Free a string returned by `minigraf_execute`.
void minigraf_string_free(char *s);

// Flush the WAL to the database file. Returns 0 on success, -1 on error.
int minigraf_checkpoint(struct MiniGrafDb *handle);

// Return the last error message. Valid until the next call on the same handle.
// Returns NULL if no error has occurred.
const char *minigraf_last_error(struct MiniGrafDb *handle);

#endif  /* MINIGRAF_H */
