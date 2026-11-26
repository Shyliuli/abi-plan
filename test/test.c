#include "../src/abi_plan.h"
#include <stdio.h>

int main(void) {
    CType i32 = { .tag = CType_Int, .int_ = { .len = 4} };
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
    CallPlan plan;
    int result=abi_plan_gen(f,&plan);
    if (result!=0){
        fprintf(stderr,"%s\n",abi_plan_error());
        return -1;
    }
    (void)plan;
    return 0;
}