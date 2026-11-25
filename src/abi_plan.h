#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * 寄存器，后续可扩展为按 ABI 返回不同的寄存器。
 */
typedef enum Register {
  Register_Rax,
  Register_Rcx,
  Register_Rdx,
  Register_Rbx,
  Register_Rsp,
  Register_Rbp,
  Register_Rsi,
  Register_Rdi,
  Register_R8,
  Register_R9,
  Register_Xmm0,
  Register_Xmm1,
  Register_Xmm2,
  Register_Xmm3,
  Register_Xmm4,
  Register_Xmm5,
  Register_Xmm6,
  Register_Xmm7,
} Register;

/**
 * 分配方式：寄存器、栈或 sret。
 */
typedef enum Allocation_Tag {
  Allocation_Void,
  Allocation_Reg,
  Allocation_Stack,
  Allocation_Sret,
} Allocation_Tag;

typedef struct Allocation_Reg_Body {
  enum Register *reg;
  uintptr_t len;
} Allocation_Reg_Body;

typedef struct Allocation_Stack_Body {
  uintptr_t offset;
} Allocation_Stack_Body;

typedef struct Allocation {
  Allocation_Tag tag;
  union {
    Allocation_Reg_Body reg;
    Allocation_Stack_Body stack;
  };
} Allocation;

typedef struct Place {
  uintptr_t size;
  uintptr_t align;
  struct Allocation alloc;
} Place;

/**
 * 调用计划，作为 abi_plan 库的计算结果，指导外部程序进行 ABI lowering。
 *
 * `args` 与各 Reg 数组由 malloc 分配，用完后通过 `free_plan` 释放。
 */
typedef struct CallPlan {
  struct Place ret;
  struct Place *args;
  uintptr_t args_len;
  uintptr_t stack_size;
} CallPlan;

/**
 * C 类型。
 *
 * 指针类型用 `Int { len: 8, .. }` 表示；`Void` 用于返回值声明，
 * 空参数函数可以是 `len == 1 && *args == Void` 或 `len == 0`。
 */
enum CType_Tag
#if __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // __STDC_VERSION__ >= 202311L
 {
  CType_Int,
  CType_Float,
  CType_Void,
  CType_Struct,
  CType_Array,
  CType_Union,
};
#if __STDC_VERSION__ >= 202311L
typedef enum CType_Tag CType_Tag;
#else
typedef uint32_t CType_Tag;
#endif // __STDC_VERSION__ >= 202311L

typedef struct CType_Int_Body {
  uintptr_t len;
  bool is_signed;
} CType_Int_Body;

typedef struct CType_Float_Body {
  uintptr_t len;
} CType_Float_Body;

typedef struct CType_Struct_Body {
  struct CType *ptr;
  uintptr_t len;
} CType_Struct_Body;

typedef struct CType_Array_Body {
  struct CType *item;
  uintptr_t len;
} CType_Array_Body;

typedef struct CType_Union_Body {
  struct CType *ptr;
  uintptr_t len;
} CType_Union_Body;

typedef struct CType {
  CType_Tag tag;
  union {
    CType_Int_Body int_;
    CType_Float_Body float_;
    CType_Struct_Body struct_;
    CType_Array_Body array;
    CType_Union_Body union_;
  };
} CType;

/**
 * CFunc C 函数声明。
 *
 * - `args` 参数列表，`args_len` 为其长度，`ret` 为返回值类型。
 * 由外部程序构造后作为请求发送。
 */
typedef struct CFunc {
  struct CType *args;
  uint32_t args_len;
  struct CType ret;
} CFunc;

struct CallPlan abi_plan_gen(struct CFunc f);
