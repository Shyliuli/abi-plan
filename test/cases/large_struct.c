// args: struct Large
// result: int
// input: {{1, 2, 3, 4, 5, 6, 7, 8}}
// expected: 1

typedef struct {
    int values[8];
} Large;

int large_struct(Large value) {
    return value.values[0];
}
