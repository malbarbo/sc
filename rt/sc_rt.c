// libsc_rt: the handlers of UBSan's checks (PLANO-VERIFICACOES.md §5.1) and
// of libc++'s hardening (§5.3), for the code sc compiles at the levels on and
// strict of --safety. A report says what went wrong, in UBSan's words, and
// stops the program with a trap: where it went wrong are the frames the host
// shows (sc, which knows a report by __sc_rt_report) or wasmtime's backtrace.
// Only the handlers that do not recover (-fno-sanitize-recover), in the
// layouts of compiler-rt's ubsan_handlers.h, for wasm32.
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef uintptr_t Value; // a value as wide as a pointer, else a pointer to it

typedef struct {
  const char *file;
  uint32_t line, column;
} Location;

typedef struct {
  uint16_t kind; // 0: an integer, 1: a float, else unknown
  uint16_t info; // an integer's log2(width) << 1 | signed; a float's width
  char name[];   // in quotes: 'int'
} Type;

__attribute__((noinline)) _Noreturn void __sc_rt_report(const char *msg) {
  fflush(stdout); // what the program printed comes first
  fprintf(stderr, "error: %s\n", msg);
  __builtin_trap();
}

static _Noreturn void report(const char *fmt, ...) __attribute__((format(printf, 1, 2)));

// snprintf, which the runtime wraps (sc_asan.c): its own calls would come
// back to it
static int format(char *buf, size_t n, const char *fmt, ...) __attribute__((format(printf, 3, 4)));

static int format(char *buf, size_t n, const char *fmt, ...) {
  __builtin_va_list ap;
  __builtin_va_start(ap, fmt);
  int r = vsnprintf(buf, n, fmt, ap);
  __builtin_va_end(ap);
  return r;
}

static _Noreturn void report(const char *fmt, ...) {
  char msg[512];
  __builtin_va_list ap;
  __builtin_va_start(ap, fmt);
  vsnprintf(msg, sizeof msg, fmt, ap);
  __builtin_va_end(ap);
  __sc_rt_report(msg);
}

static unsigned width(const Type *t) {
  return t->kind == 0 ? 1u << (t->info >> 1) : t->kind == 1 ? t->info : 0;
}

static int is_signed(const Type *t) { return t->kind == 0 && (t->info & 1); }

static int inline_value(const Type *t) { return width(t) <= 8 * sizeof(Value); }

// an integer, as wide as 128 bits
static __int128 integer(const Type *t, Value v) {
  unsigned w = width(t);
  if (!inline_value(t)) {
    if (w == 64) return is_signed(t) ? (__int128)*(int64_t *)v : (__int128)*(uint64_t *)v;
    return is_signed(t) ? *(__int128 *)v : (__int128)*(unsigned __int128 *)v;
  }
  uint64_t x = v;
  if (is_signed(t) && w < 64) {
    uint64_t sign = 1ull << (w - 1);
    x = (x & ((sign << 1) - 1)) ^ sign;
    return (int64_t)(x - sign);
  }
  return x;
}

// v as the source writes it, in buf
static const char *text(char *buf, size_t n, const Type *t, Value v) {
  if (t->kind == 1) {
    double d = width(t) == 32 ? (double)*(float *)&v
             : width(t) == 64 ? *(double *)v
             : (double)*(long double *)v;
    format(buf, n, "%g", d);
    return buf;
  }
  if (t->kind != 0) return "<unknown>";
  __int128 x = integer(t, v);
  if (x >= INT64_MIN && x <= INT64_MAX) {
    format(buf, n, "%lld", (long long)x);
  } else if (x >= 0 && (unsigned __int128)x <= UINT64_MAX) {
    format(buf, n, "%llu", (unsigned long long)x);
  } else { // 128 bits
    char d[48];
    int k = sizeof d;
    unsigned __int128 u = x < 0 ? -(unsigned __int128)x : (unsigned __int128)x;
    d[--k] = 0;
    do d[--k] = '0' + (char)(u % 10); while (u /= 10);
    if (x < 0) d[--k] = '-';
    format(buf, n, "%s", d + k);
  }
  return buf;
}

static int negative(const Type *t, Value v) { return is_signed(t) && integer(t, v) < 0; }

// --- arithmetic

typedef struct {
  Location loc;
  const Type *type;
} OverflowData;

static _Noreturn void overflow(OverflowData *d, Value l, Value r, char op) {
  char a[48], b[48];
  report("%s integer overflow: %s %c %s cannot be represented in type %s", is_signed(d->type) ? "signed" : "unsigned",
         text(a, sizeof a, d->type, l), op, text(b, sizeof b, d->type, r), d->type->name);
}

void __ubsan_handle_add_overflow_abort(OverflowData *d, Value l, Value r) { overflow(d, l, r, '+'); }
void __ubsan_handle_sub_overflow_abort(OverflowData *d, Value l, Value r) { overflow(d, l, r, '-'); }
void __ubsan_handle_mul_overflow_abort(OverflowData *d, Value l, Value r) { overflow(d, l, r, '*'); }

void __ubsan_handle_negate_overflow_abort(OverflowData *d, Value v) {
  char a[48];
  report("negation of %s cannot be represented in type %s%s", text(a, sizeof a, d->type, v), d->type->name,
         is_signed(d->type) ? "; cast to an unsigned type to negate this value to itself" : "");
}

void __ubsan_handle_divrem_overflow_abort(OverflowData *d, Value l, Value r) {
  char a[48];
  if (is_signed(d->type) && integer(d->type, r) == -1)
    report("division of %s by -1 cannot be represented in type %s", text(a, sizeof a, d->type, l), d->type->name);
  report("division by zero");
}

typedef struct {
  Location loc;
  const Type *lhs, *rhs;
} ShiftData;

void __ubsan_handle_shift_out_of_bounds_abort(ShiftData *d, Value l, Value r) {
  char a[48], b[48];
  text(a, sizeof a, d->lhs, l);
  text(b, sizeof b, d->rhs, r);
  if (negative(d->rhs, r))
    report("shift exponent %s is negative", b);
  if (integer(d->rhs, r) >= width(d->lhs))
    report("shift exponent %s is too large for %u-bit type %s", b, width(d->lhs), d->lhs->name);
  if (negative(d->lhs, l))
    report("left shift of negative value %s", a);
  report("left shift of %s by %s places cannot be represented in type %s", a, b, d->lhs->name);
}

typedef struct {
  Location loc;
  const Type *from, *to;
  uint8_t kind;      // 0, 1, 2: a truncation; 3, 4: a change of sign
  uint32_t bitfield; // the bit-field's width, or 0
} ImplicitConversionData;

void __ubsan_handle_implicit_conversion_abort(ImplicitConversionData *d, Value from, Value to) {
  char a[48], b[48];
  unsigned w = d->bitfield ? d->bitfield : width(d->to);
  report("implicit conversion from type %s of value %s (%u-bit, %ssigned) to type %s changed the value to %s "
         "(%u-bit%s, %ssigned)", d->from->name, text(a, sizeof a, d->from, from), width(d->from),
         is_signed(d->from) ? "" : "un", d->to->name, text(b, sizeof b, d->to, to), w,
         d->bitfield ? " bitfield" : "", is_signed(d->to) ? "" : "un");
}

typedef struct {
  Location loc;
  const Type *from, *to;
} FloatCastData;

void __ubsan_handle_float_cast_overflow_abort(FloatCastData *d, Value v) {
  char a[48];
  report("%s is outside the range of representable values of type %s", text(a, sizeof a, d->from, v), d->to->name);
}

// --- memory

typedef struct {
  Location loc;
  const Type *array, *index;
} OutOfBoundsData;

void __ubsan_handle_out_of_bounds_abort(OutOfBoundsData *d, Value i) {
  char a[48];
  report("index %s out of bounds for type %s", text(a, sizeof a, d->index, i), d->array->name);
}

typedef struct {
  Location loc;
  const Type *type;
  uint8_t log_alignment, check;
} TypeMismatchData;

static const char *const access[] = {
    "load of", "store to", "reference binding to", "member access within", "member call on",
    "constructor call on", "downcast of", "downcast of", "upcast of", "cast to virtual base of",
    "_Nonnull binding to", "dynamic operation on"};

// -ftrivial-auto-var-init=pattern's pointer on wasm32: a local the code
// never set
#define UNINITIALIZED UINTPTR_MAX

void __ubsan_handle_type_mismatch_v1_abort(TypeMismatchData *d, Value p) {
  const char *what = d->check < sizeof access / sizeof *access ? access[d->check] : "access to";
  uintptr_t alignment = (uintptr_t)1 << d->log_alignment;
  if (!p)
    report("%s null pointer of type %s", what, d->type->name);
  if (p == UNINITIALIZED)
    report("%s uninitialized pointer %p of type %s", what, (void *)p, d->type->name);
  if (p & (alignment - 1))
    report("%s misaligned address %p for type %s, which requires %lu byte alignment", what, (void *)p, d->type->name,
           (unsigned long)alignment);
  report("%s address %p with insufficient space for an object of type %s", what, (void *)p, d->type->name);
}

typedef struct {
  Location loc;
} PointerOverflowData;

void __ubsan_handle_pointer_overflow_abort(PointerOverflowData *d, Value base, Value result) {
  if (!base && !result)
    report("applying zero offset to null pointer");
  if (!base)
    report("applying non-zero offset %lu to null pointer", (unsigned long)result);
  if (base == UNINITIALIZED)
    report("pointer arithmetic on uninitialized pointer %p", (void *)base);
  if (!result)
    report("applying non-zero offset to non-null pointer %p produced null pointer", (void *)base);
  if (((intptr_t)base >= 0) == ((intptr_t)result >= 0))
    report("%s unsigned offset %s %p overflowed to %p", base > result ? "addition of" : "subtraction of",
           base > result ? "to" : "from", (void *)base, (void *)result);
  report("pointer index expression with base %p overflowed to %p", (void *)base, (void *)result);
}

typedef struct {
  Location loc;
  const Type *type;
} ValueData;

void __ubsan_handle_vla_bound_not_positive_abort(ValueData *d, Value v) {
  char a[48];
  report("variable length array bound evaluates to non-positive value %s", text(a, sizeof a, d->type, v));
}

void __ubsan_handle_load_invalid_value_abort(ValueData *d, Value v) {
  char a[48];
  report("load of value %s, which is not a valid value for type %s", text(a, sizeof a, d->type, v), d->type->name);
}

typedef struct {
  Location loc, assumption;
  const Type *type;
} AlignmentAssumptionData;

void __ubsan_handle_alignment_assumption_abort(AlignmentAssumptionData *d, Value p, Value alignment, Value offset) {
  if (offset)
    report("assumption of %lu byte alignment (with offset of %lu byte) for pointer of type %s failed",
           (unsigned long)alignment, (unsigned long)offset, d->type->name);
  report("assumption of %lu byte alignment for pointer of type %s failed", (unsigned long)alignment, d->type->name);
}

// --- functions

typedef struct {
  Location loc;
  uint8_t kind; // 0: ctz, 1: clz, 2: assume
} BuiltinData;

void __ubsan_handle_invalid_builtin_abort(BuiltinData *d) {
  if (d->kind == 2)
    report("assumption is violated during execution");
  report("passing zero to __builtin_%s(), which is not a valid argument", d->kind ? "clz" : "ctz");
}

typedef struct {
  Location loc, attribute;
  int arg;
} NonNullArgData;

void __ubsan_handle_nonnull_arg_abort(NonNullArgData *d) {
  report("null pointer passed as argument %d, which is declared to never be null", d->arg);
}

typedef struct {
  Location attribute;
} NonNullReturnData;

void __ubsan_handle_nonnull_return_v1_abort(NonNullReturnData *d, Location *loc) {
  report("null pointer returned from function declared to never return null");
}

typedef struct {
  Location loc;
} UnreachableData;

void __ubsan_handle_builtin_unreachable(UnreachableData *d) {
  report("execution reached an unreachable program point");
}

void __ubsan_handle_missing_return(UnreachableData *d) {
  report("execution reached the end of a value-returning function without returning a value");
}

// --- libc++'s hardening: a failed check calls std::__libcpp_verbose_abort
// with "file:line: libc++ Hardening assertion COND failed: REASON\n" (the
// sysroot's semantic is enforce). libc++ defines it weak, for a program to
// put its own in its place, as this one does: the loader and wasm-ld take
// the first definition, and libsc_rt comes before libc++

_Noreturn void verbose_abort(const char *fmt, ...) __asm__("_ZNSt3__222__libcpp_verbose_abortEPKcz");

_Noreturn void verbose_abort(const char *fmt, ...) {
  char msg[512];
  __builtin_va_list ap;
  __builtin_va_start(ap, fmt);
  vsnprintf(msg, sizeof msg, fmt, ap);
  __builtin_va_end(ap);
  char *reason = strstr(msg, " failed: ");
  reason = reason ? reason + strlen(" failed: ") : msg;
  reason[strcspn(reason, "\n")] = 0;
  __sc_rt_report(reason);
}
