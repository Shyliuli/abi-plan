#include "../src/abi_plan.h"

int main(void) {
    CType i32 = { .tag = CType_Int, .int_ = { .len = 4, .is_signed = true } };
    CType f64 = { .tag = CType_Float, .float_ = { .len = 8 } };
    CType fields[] = { i32, f64 };
    CType args[] = {
        i32,
        f64,
        { .tag = CType_Struct, .struct_ = { .ptr = fields, .len = 2 } },
        { .tag = CType_Array,  .array   = { .item = &i32,  .len = 4 } },
        { .tag = CType_Union,  .union_  = { .ptr = fields, .len = 2 } },
    };

    CFunc f = {
        .args = args,
        .args_len = sizeof(args) / sizeof(args[0]),
        .ret = { .tag = CType_Float, .float_ = { .len = 4 } },
    };
    CallPlan plan = abi_plan_gen(f);
    (void)plan;
    return 0;
}