// args: struct Outer
// result: int
// input: {{19, 20}, 3}
// expected: 42

typedef struct {
    int x;
    int y;
} Pair;

typedef struct {
    Pair pair;
    int extra;
} Outer;

int nested_struct(Outer value) {
    return value.pair.x + value.pair.y + value.extra;
}
