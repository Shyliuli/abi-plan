// args: struct Pair
// result: int
// input: {19, 23}
// expected: 42

typedef struct {
    int x;
    int y;
} Pair;

int small_struct(Pair value) {
    return value.x + value.y;
}
