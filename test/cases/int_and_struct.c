// args: int, struct Pair
// result: int
// input: 40, {1, 1}
// expected: 42

typedef struct {
    int x;
    int y;
} Pair;

int int_and_struct(int base, Pair value) {
    return base + value.x + value.y;
}
