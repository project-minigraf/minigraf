#include "minigraf.h"
#include <stdio.h>
#include <stdlib.h>

int main(void) {
    MiniGrafDb *db = minigraf_open_in_memory();
    if (!db) {
        fprintf(stderr, "failed to open database\n");
        return 1;
    }

    char *result = minigraf_execute(db, "(transact [[:alice :name \"Alice\"] [:alice :age 30]])");
    if (!result) {
        fprintf(stderr, "transact error: %s\n", minigraf_last_error(db));
        minigraf_close(db);
        return 1;
    }
    minigraf_string_free(result);

    char *query = minigraf_execute(db, "(query [:find ?e ?name :where [?e :name ?name]])");
    if (!query) {
        fprintf(stderr, "query error: %s\n", minigraf_last_error(db));
        minigraf_close(db);
        return 1;
    }
    printf("%s\n", query);
    minigraf_string_free(query);

    minigraf_checkpoint(db);
    minigraf_close(db);
    return 0;
}
