// libsc_rt: AddressSanitizer's runtime (PLANO-VERIFICACOES.md §5.2), for the
// code sc compiles at the levels on and strict of --safety. A report says
// what went wrong and stops the program through __sc_rt_report (sc_rt.c).
//
// The memory: the stack first, in [16, 1 MiB); the shadow at SHADOW, 512 MiB,
// one byte for each 8 of the 4 GiB; the program after it. A shadow byte of 0
// means its 8 bytes are addressable, k in 1..7 the first k, and a byte with
// the high bit set none, with what they are (the magic numbers below, as
// compiler-rt's).
//
// The allocator wraps libc's: each block has a left redzone with its header
// and a right redzone, and a freed block waits in a quarantine before it goes
// back to libc. The functions it replaces, and those it checks the arguments
// of, are named as wasm-ld's --wrap names them: __wrap_malloc is the malloc
// of the code linked with libsc_rt, and __real_malloc libc's. The static
// programs are linked with --wrap; the REPL's loader does the same for the
// shared objects. libc calls its own malloc, so a block that libc allocates
// (strdup) has no header, and free passes it on. The live blocks are in a
// list, for __sc_rt_leaks.
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <sys/types.h>

#define SHADOW 0x110000u // scripts/flags.py
#define QUARANTINE (64u << 20)

_Noreturn void __sc_rt_report(const char *msg); // sc_rt.c

enum {
  HEAP_LEFT = 0xfa, HEAP_RIGHT = 0xfb, FREED = 0xfd, CONTAINER = 0xfc,
  STACK_LEFT = 0xf1, STACK_MID = 0xf2, STACK_RIGHT = 0xf3, STACK_SCOPE = 0xf8,
  GLOBAL = 0xf9, ALLOCA_LEFT = 0xca, ALLOCA_RIGHT = 0xcb, NULL_PAGE = 0xfe,
};

// snprintf, which the runtime wraps: its own calls would come back to it
static int format(char *buf, size_t n, const char *fmt, ...) __attribute__((format(printf, 3, 4)));

static int format(char *buf, size_t n, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = vsnprintf(buf, n, fmt, ap);
  va_end(ap);
  return r;
}

static inline uint8_t *shadow(uintptr_t a) { return (uint8_t *)((a >> 3) + SHADOW); }

static inline uintptr_t up8(uintptr_t a) { return (a + 7) & ~(uintptr_t)7; }

// [a, a + n), a and n multiples of 8, all v
static void fill(uintptr_t a, size_t n, uint8_t v) { memset(shadow(a), v, n / 8); }

// [a, a + n) addressable, a a multiple of 8; the last granule partly
static void unpoison(uintptr_t a, size_t n) {
  memset(shadow(a), 0, n / 8);
  if (n % 8) *shadow(a + n - n % 8) = n % 8;
}

// the first byte of [a, a + n) not addressable, or 0
static uintptr_t first_bad(uintptr_t a, size_t n) {
  if (a + n < a) return a;
  for (uintptr_t p = a; p < a + n;) {
    int8_t v = (int8_t)*shadow(p);
    if (v == 0) p = (p | 7) + 1;
    else if (v > 0 && (int)(p & 7) < v) p++;
    else return p ? p : 1; // 0 stands for none: the byte at 0 is at 1's granule
  }
  return 0;
}

// what the byte at a is: a partly addressable granule is what follows it
static uint8_t kind(uintptr_t a) {
  uint8_t v = *shadow(a);
  return v > 0 && v < 8 ? *shadow(a + 8) : v;
}

// --- the stack, the globals, the heap: what an address is

extern char __stack_low, __stack_high; // the loader's, wasm-ld's

static int in_stack(uintptr_t a) { return a >= (uintptr_t)&__stack_low && a < (uintptr_t)&__stack_high; }

typedef struct {
  uintptr_t beg, size, size_with_redzone;
  const char *name, *module_name;
  uintptr_t has_dynamic_init;
  void *location;
  uintptr_t odr_indicator;
} Global;

// the arrays __asan_register_globals got, each module's
static struct { Global *g; uintptr_t n; } *globals;
static size_t nglobals, capglobals;

static const Global *global_at(uintptr_t a) {
  for (size_t i = 0; i < nglobals; i++)
    for (uintptr_t k = 0; k < globals[i].n; k++) {
      const Global *g = &globals[i].g[k];
      if (a >= g->beg && a < g->beg + g->size_with_redzone) return g;
    }
  return 0;
}

#define LIVE 0x5c11feedu
#define DEAD 0x5cdeadu

// the header, just before the address the program gets
typedef struct Block {
  uint32_t magic, size; // LIVE or DEAD; the size the program asked for
  uint32_t left, total; // the block libc gave: user - left, total bytes
  struct Block *next, *prev; // in the list of the live blocks; next in the quarantine
  uint32_t allocated, freed; // where: the numbers of their stacks (stack)
} Block;

_Static_assert(sizeof(Block) == 32, "the header is 4 granules");

// the loader's (sc): __sc_rt_stack keeps the stack of its caller under a
// number, and __sc_rt_blame names the stacks of the block of the report
// that follows. Weak: a program run without them (the corpus's) has none
__attribute__((weak)) uint32_t __sc_rt_stack(void);
__attribute__((weak)) void __sc_rt_blame(uint32_t allocated, uint32_t freed);

// Only the first STACKS blocks keep their stacks: each costs a few
// microseconds, and a std::map of 100 thousand keys took 5 times as long
// with all of them (ANALISE.md)
#define STACKS 20000
static uint32_t stacks;

static uint32_t stack(void) { return __sc_rt_stack ? __sc_rt_stack() : 0; }

static void blame(const Block *b) {
  if (__sc_rt_blame) __sc_rt_blame(b->allocated, b->magic == DEAD ? b->freed : 0);
}

static Block *header(uintptr_t user) { return (Block *)(user - sizeof(Block)); }

// the block whose memory the program gets at user, if it is one of the
// allocator's: its left redzone is before it
static Block *block(uintptr_t user) {
  if (user % 8 || user < SHADOW + (512u << 20) || *shadow(user - 8) != HEAP_LEFT) return 0;
  Block *b = header(user);
  return b->magic == LIVE || b->magic == DEAD ? b : 0;
}

// the block a, in a heap redzone, a freed block or a container's spare room,
// is next to or in, and where it starts; looking back at most limit granules
static Block *block_near(uintptr_t a, uint8_t k, uintptr_t *user, int limit) {
  uintptr_t g = a & ~(uintptr_t)7;
  if (k == HEAP_LEFT) { // before a block
    while (*shadow(g) == HEAP_LEFT) g += 8;
  } else {
    if (k == HEAP_RIGHT) { // after one
      while (g >= 8 && *shadow(g) == HEAP_RIGHT) g -= 8;
      if (*shadow(g) == HEAP_LEFT) g += 8; // of 0 bytes
    }
    for (int i = 0; g >= 8 && *shadow(g - 8) != HEAP_LEFT; i++, g -= 8)
      if (i == limit || *shadow(g - 8) == HEAP_RIGHT) return 0;
  }
  *user = g;
  return block(g);
}

// a stack frame: the pass puts at its start a magic number, a description
// of its variables and the function
#define FRAME_MAGIC 0x41B58AB3u

// the frame a, in a stack redzone, is in, and its description
static const char *frame_at(uintptr_t a, uintptr_t *base) {
  uintptr_t g = a & ~(uintptr_t)7;
  for (int i = 0; i < (1 << 17) && g >= 8 && *shadow(g) != STACK_LEFT; i++) g -= 8;
  while (g >= 8 && *shadow(g - 8) == STACK_LEFT) g -= 8;
  if (*shadow(g) != STACK_LEFT || *(uint32_t *)g != FRAME_MAGIC) return 0;
  *base = g;
  return ((const char **)g)[1];
}

// the variable of the frame at base nearest a: "n (offset size len name[:line])..."
static int frame_var(const char *d, uintptr_t base, uintptr_t a, uintptr_t *beg, uintptr_t *size, char *name,
                     size_t cap) {
  char *e;
  long n = strtol(d, &e, 10);
  long best = -1, bestd = 0;
  for (long i = 0; i < n; i++) {
    long off = strtol(e, &e, 10), sz = strtol(e, &e, 10), len = strtol(e, &e, 10);
    const char *nm = e + 1;
    e = (char *)nm + len;
    uintptr_t vb = base + off, ve = vb + sz;
    long dist = a < vb ? (long)(vb - a) : a >= ve ? (long)(a - ve + 1) : 0;
    if (best < 0 || dist < bestd) {
      best = i, bestd = dist, *beg = vb, *size = sz;
      size_t m = (size_t)len < cap - 1 ? (size_t)len : cap - 1;
      memcpy(name, nm, m);
      name[m] = 0;
    }
  }
  return best >= 0;
}

// --- the reports

static const char *bytes(char *buf, size_t n, uintptr_t k) {
  format(buf, n, "%lu byte%s", (unsigned long)k, k == 1 ? "" : "s");
  return buf;
}

// "N bytes past the end of", "N bytes before the start of", "at the start
// of", "N bytes into"
static const char *where(char *buf, size_t n, uintptr_t a, uintptr_t beg, uintptr_t size) {
  char b[32];
  if (a >= beg + size && size) format(buf, n, "%s past the end of", bytes(b, sizeof b, a - beg - size));
  else if (a < beg) format(buf, n, "%s before the start of", bytes(b, sizeof b, beg - a));
  else if (a == beg) format(buf, n, "at the start of");
  else format(buf, n, "%s into", bytes(b, sizeof b, a - beg));
  return buf;
}

static void name_line(char *name) { // "a:6" is the variable a, at line 6
  char *c = strrchr(name, ':');
  if (c) *c = 0;
}

// the report of an access of n bytes at a, whose first bad byte is bad;
// fn: the function of libc that does it; room: n is the room it is told of
// (fgets), which this input may not fill
static _Noreturn void access_error(uintptr_t a, uintptr_t n, int write, uintptr_t bad, const char *fn, int room) {
  char msg[512], acc[96], w[64], b[32], name[128];
  uint8_t k = kind(bad);
  format(acc, sizeof acc, "%s of %s%s%s%s", write ? "write" : "read", room ? "up to " : "", bytes(b, sizeof b, n),
         fn ? " by " : "", fn ? fn : "");
  uintptr_t user, beg, size;
  switch (k) {
  case HEAP_LEFT: case HEAP_RIGHT: case FREED: case CONTAINER: {
    Block *blk = block_near(bad, k, &user, 1 << 20);
    if (!blk) break;
    if (k == CONTAINER)
      format(msg, sizeof msg, "container-overflow: %s, past the size of a container (std::vector or std::deque)",
               acc);
    else if (k == FREED)
      format(msg, sizeof msg, "heap-use-after-free: %s, %s a freed block of %s", acc,
               where(w, sizeof w, bad, user, blk->size), bytes(b, sizeof b, blk->size));
    else
      format(msg, sizeof msg, "heap-buffer-overflow: %s, %s a block of %s", acc,
               where(w, sizeof w, bad, user, blk->size), bytes(b, sizeof b, blk->size));
    blame(blk);
    __sc_rt_report(msg);
  }
  case STACK_LEFT: case STACK_MID: case STACK_RIGHT: case STACK_SCOPE: {
    uintptr_t base;
    const char *d = frame_at(bad, &base);
    if (!d || !frame_var(d, base, bad, &beg, &size, name, sizeof name)) break;
    name_line(name);
    if (k == STACK_SCOPE)
      format(msg, sizeof msg, "stack-use-after-scope: %s of the variable '%s', after its scope ended", acc, name);
    else
      format(msg, sizeof msg, "stack-buffer-overflow: %s, %s the variable '%s' (%s)", acc,
               where(w, sizeof w, bad, beg, size), name, bytes(b, sizeof b, size));
    __sc_rt_report(msg);
  }
  case GLOBAL: {
    const Global *g = global_at(bad);
    if (!g) break;
    format(msg, sizeof msg, "global-buffer-overflow: %s, %s the global '%s' (%s)", acc,
             where(w, sizeof w, bad, g->beg, g->size), g->name, bytes(b, sizeof b, g->size));
    __sc_rt_report(msg);
  }
  case ALLOCA_LEFT: case ALLOCA_RIGHT: {
    uintptr_t g = bad & ~(uintptr_t)7, e;
    if (k == ALLOCA_RIGHT) {
      while (*shadow(g) == ALLOCA_RIGHT) g -= 8;
      e = g + (*shadow(g) ? *shadow(g) : 8);
      while (*shadow(g - 8) != ALLOCA_LEFT) g -= 8;
    } else {
      while (*shadow(g) == ALLOCA_LEFT) g += 8;
      for (e = g; *shadow(e) == 0; e += 8) {}
      e += *shadow(e) < 8 ? *shadow(e) : 0;
    }
    format(msg, sizeof msg, "dynamic-stack-buffer-overflow: %s, %s a variable-length array (%s)", acc,
             where(w, sizeof w, bad, g, e - g), bytes(b, sizeof b, e - g));
    __sc_rt_report(msg);
  }
  case NULL_PAGE:
    format(msg, sizeof msg, "%s through a null pointer", acc);
    __sc_rt_report(msg);
  }
  format(msg, sizeof msg, "invalid memory access: %s at %p", acc, (void *)bad);
  __sc_rt_report(msg);
}

static void check(uintptr_t a, uintptr_t n, int write, const char *fn) {
  uintptr_t bad = first_bad(a, n);
  if (bad) access_error(a, n, write, bad, fn, 0);
}

static void check_room(uintptr_t a, uintptr_t n, const char *fn) {
  uintptr_t bad = first_bad(a, n);
  if (bad) access_error(a, n, 1, bad, fn, 1);
}

static void check_str(const char *s, int write, const char *fn) { // a string and its 0
  check((uintptr_t)s, strlen(s) + 1, write, fn);
}

#define REPORT(n)                                                                                                     \
  void __asan_report_load##n(uintptr_t a) { access_error(a, n, 0, first_bad(a, n) ?: a, 0, 0); }                      \
  void __asan_report_store##n(uintptr_t a) { access_error(a, n, 1, first_bad(a, n) ?: a, 0, 0); }                     \
  void __asan_load##n(uintptr_t a) { check(a, n, 0, 0); }                                                            \
  void __asan_store##n(uintptr_t a) { check(a, n, 1, 0); }
REPORT(1)
REPORT(2)
REPORT(4)
REPORT(8)
REPORT(16)

void __asan_report_load_n(uintptr_t a, uintptr_t n) { access_error(a, n, 0, first_bad(a, n) ?: a, 0, 0); }
void __asan_report_store_n(uintptr_t a, uintptr_t n) { access_error(a, n, 1, first_bad(a, n) ?: a, 0, 0); }
void __asan_loadN(uintptr_t a, uintptr_t n) { check(a, n, 0, 0); }
void __asan_storeN(uintptr_t a, uintptr_t n) { check(a, n, 1, 0); }

// --- what the pass calls

static int inited;

void __asan_init(void) {
  if (inited) return;
  inited = 1;
  fill(0, 16, NULL_PAGE); // the 16 bytes below the stack
}

void __asan_version_mismatch_check_v8(void) {}
void __asan_before_dynamic_init(const char *module) {}
void __asan_after_dynamic_init(void) {}

// a frame's shadow, larger than the pass writes inline: a is a shadow address
#define SET_SHADOW(v)                                                                                                 \
  void __asan_set_shadow_##v(uintptr_t a, uintptr_t n) { memset((void *)a, 0x##v, n); }
SET_SHADOW(00)
SET_SHADOW(01)
SET_SHADOW(02)
SET_SHADOW(03)
SET_SHADOW(04)
SET_SHADOW(05)
SET_SHADOW(06)
SET_SHADOW(07)
SET_SHADOW(f1)
SET_SHADOW(f2)
SET_SHADOW(f3)
SET_SHADOW(f5)
SET_SHADOW(f8)

// a variable-length array or an alloca: redzones of 32 bytes around it
void __asan_alloca_poison(uintptr_t a, uintptr_t n) {
  uintptr_t end = a + n, right = (end + 31) & ~(uintptr_t)31, partial = end & ~(uintptr_t)7;
  fill(a - 32, 32, ALLOCA_LEFT);
  for (uintptr_t g = partial; g < right; g += 8)
    *shadow(g) = g == partial && end % 8 ? end % 8 : ALLOCA_RIGHT;
  fill(right, 32, ALLOCA_RIGHT);
}

void __asan_allocas_unpoison(uintptr_t top, uintptr_t bottom) {
  if (top && top <= bottom) memset(shadow(top), 0, (bottom - top) / 8);
}

// a function that does not return (a throw, longjmp, exit): the frames it
// leaves will not clean their shadow
void __asan_handle_no_return(void) {
  uintptr_t sp = (uintptr_t)__builtin_frame_address(0) & ~(uintptr_t)7, top = (uintptr_t)&__stack_high;
  if (in_stack(sp)) memset(shadow(sp), 0, (top - sp) / 8);
}

void *__asan_memcpy(void *d, const void *s, size_t n) {
  check((uintptr_t)s, n, 0, "memcpy");
  check((uintptr_t)d, n, 1, "memcpy");
  if (n && (char *)d < (const char *)s + n && (const char *)s < (char *)d + n)
    __sc_rt_report("memcpy-param-overlap: the source and the destination of memcpy overlap (use memmove)");
  return memcpy(d, s, n);
}

void *__asan_memmove(void *d, const void *s, size_t n) {
  check((uintptr_t)s, n, 0, "memmove");
  check((uintptr_t)d, n, 1, "memmove");
  return memmove(d, s, n);
}

void *__asan_memset(void *d, int c, size_t n) {
  check((uintptr_t)d, n, 1, "memset");
  return memset(d, c, n);
}

void *__real_malloc(size_t);
void __real_free(void *);

void __asan_register_globals(Global *g, uintptr_t n) {
  for (uintptr_t i = 0; i < n; i++) {
    unpoison(g[i].beg, g[i].size);
    uintptr_t rz = up8(g[i].beg + g[i].size);
    fill(rz, g[i].beg + g[i].size_with_redzone - rz, GLOBAL);
  }
  if (nglobals == capglobals) {
    size_t cap = capglobals ? 2 * capglobals : 64;
    void *p = __real_malloc(cap * sizeof *globals);
    if (!p) return;
    if (globals) memcpy(p, globals, nglobals * sizeof *globals), __real_free(globals);
    globals = p, capglobals = cap;
  }
  globals[nglobals].g = g, globals[nglobals++].n = n;
}

void __asan_unregister_globals(Global *g, uintptr_t n) {
  for (uintptr_t i = 0; i < n; i++) unpoison(g[i].beg, g[i].size_with_redzone);
  for (size_t i = 0; i < nglobals; i++)
    if (globals[i].g == g) {
      globals[i] = globals[--nglobals];
      break;
    }
}

// what a program may call (sanitizer/asan_interface.h)
void __asan_poison_memory_region(const volatile void *p, size_t n) {
  uintptr_t a = (uintptr_t)p, b = a + n;
  for (uintptr_t g = up8(a); g + 8 <= b; g += 8) *shadow(g) = 0xf7;
}

void __asan_unpoison_memory_region(const volatile void *p, size_t n) {
  uintptr_t a = (uintptr_t)p & ~(uintptr_t)7, b = (uintptr_t)p + n;
  memset(shadow(a), 0, (b - a) / 8);
  if ((b - a) % 8 && *shadow(b & ~(uintptr_t)7) != 0) *shadow(b & ~(uintptr_t)7) = (b - a) % 8;
}

// --- libc++'s containers: the room past a vector's size, or around a
// deque's elements, is not addressable (container-overflow). beg and end are
// the storage's; past end, nothing is addressable (the allocator's redzone)
static void container(uintptr_t beg, uintptr_t end, uintptr_t lo, uintptr_t hi) {
  for (uintptr_t g = beg & ~(uintptr_t)7; g < end; g += 8) {
    uintptr_t s = g < beg ? beg : g, e = g + 8 < end ? g + 8 : end;
    uintptr_t used = lo < e && hi > s ? (hi < e ? hi : e) - g : g < beg ? beg - g : 0;
    *shadow(g) = used >= 8 ? 0 : used ? (uint8_t)used : CONTAINER;
  }
}

void __sanitizer_annotate_contiguous_container(const void *beg, const void *end, const void *old_mid,
                                               const void *new_mid) {
  container((uintptr_t)beg, (uintptr_t)end, (uintptr_t)beg, (uintptr_t)new_mid);
}

void __sanitizer_annotate_double_ended_contiguous_container(const void *beg, const void *end,
                                                             const void *old_beg, const void *old_end,
                                                             const void *new_beg, const void *new_end) {
  container((uintptr_t)beg, (uintptr_t)end, (uintptr_t)new_beg, (uintptr_t)new_end);
}

int __sanitizer_verify_contiguous_container(const void *beg, const void *mid, const void *end) { return 1; }

int __sanitizer_verify_double_ended_contiguous_container(const void *beg, const void *con_beg,
                                                          const void *con_end, const void *end) {
  return 1;
}

// --- the allocator

void *__real_aligned_alloc(size_t, size_t);
void *__real_realloc(void *, size_t);
size_t __real_malloc_usable_size(void *);

static Block *quarantine, *quarantine_end;
static size_t quarantined;
static Block *live; // the blocks not freed, the newest first (__sc_rt_leaks)

// the right redzone of a block of n bytes, as compiler-rt's
static uintptr_t redzone(size_t n) {
  return n <= 64 - 16 ? 16 : n <= 128 - 32 ? 32 : n <= 512 - 64 ? 64 : n <= 4096 - 128 ? 128
       : n <= (1 << 14) - 256 ? 256 : n <= (1 << 15) - 512 ? 512 : n <= (1 << 16) - 1024 ? 1024 : 2048;
}

static void *alloc(size_t n, size_t align) {
  align = align < 16 ? 16 : align;
  uintptr_t left = align < sizeof(Block) ? sizeof(Block) : align, total = left + up8(n) + redzone(n);
  if (n > 0xC0000000u) return 0;
  uintptr_t real = (uintptr_t)__real_aligned_alloc(align, total);
  if (!real) return 0;
  uintptr_t user = real + left;
  fill(real, left, HEAP_LEFT);
  unpoison(user, n);
  fill(up8(user + n), real + total - up8(user + n), HEAP_RIGHT);
  Block *b = header(user);
  *b = (Block){.magic = LIVE, .size = n, .left = left, .total = total, .next = live,
               .allocated = stacks < STACKS ? (stacks++, stack()) : 0};
  if (live) live->prev = b;
  live = b;
  return (void *)user;
}

static _Noreturn void bad_free(const char *fn, uintptr_t p) {
  char msg[256], w[64], b[32];
  uintptr_t user;
  const Global *g;
  Block *blk = block_near(p, FREED, &user, 1 << 20);
  if (blk && p - user < blk->size) {
    format(msg, sizeof msg, "invalid-free: %s of an address %s a block of %s", fn, where(w, sizeof w, p, user, blk->size),
             bytes(b, sizeof b, blk->size));
    blame(blk);
  }
  else if (in_stack(p))
    format(msg, sizeof msg, "invalid-free: %s of the address of a local variable", fn);
  else if ((g = global_at(p)))
    format(msg, sizeof msg, "invalid-free: %s of the address of the global '%s'", fn, g->name);
  else
    format(msg, sizeof msg, "invalid-free: %s of an address that malloc did not return", fn);
  __sc_rt_report(msg);
}

// the block at p, which the program gives back with fn: 0 if libc's
static Block *owned(void *p, const char *fn) {
  uintptr_t a = (uintptr_t)p, user;
  Block *b = block(a);
  if (b && b->magic == DEAD) {
    char msg[128], s[32];
    format(msg, sizeof msg, "double-free: %s of a block of %s already freed", fn, bytes(s, sizeof s, b->size));
    blame(b);
    __sc_rt_report(msg);
  }
  if (b) return b;
  if (in_stack(a) || global_at(a)) bad_free(fn, a);
  // an address inside one of the allocator's blocks, up to 64 KiB in
  if ((b = block_near(a, FREED, &user, 8192)) && a - user < b->size) bad_free(fn, a);
  return 0;
}

static void release(Block *b) {
  uintptr_t user = (uintptr_t)b + sizeof(Block);
  b->magic = DEAD;
  b->freed = b->allocated ? stack() : 0;
  if (b->prev) b->prev->next = b->next;
  else live = b->next;
  if (b->next) b->next->prev = b->prev;
  fill(user, up8(b->size), FREED);
  b->next = 0;
  if (quarantine_end) quarantine_end->next = b;
  else quarantine = b;
  quarantine_end = b;
  for (quarantined += b->total; quarantined > QUARANTINE && quarantine;) {
    Block *q = quarantine;
    uintptr_t real = (uintptr_t)q + sizeof(Block) - q->left;
    quarantine = q->next;
    if (!quarantine) quarantine_end = 0;
    quarantined -= q->total;
    q->magic = 0;
    fill(real, q->total & ~(uintptr_t)7, 0);
    __real_free((void *)real);
  }
}

void *__wrap_malloc(size_t n) { return alloc(n, 16); }

void __wrap_free(void *p) {
  if (!p) return;
  Block *b = owned(p, "free");
  if (b) release(b);
  else __real_free(p);
}

void *__wrap_calloc(size_t n, size_t m) {
  if (m && n > SIZE_MAX / m) return 0;
  void *p = alloc(n * m, 16);
  return p ? memset(p, 0, n * m) : 0;
}

void *__wrap_realloc(void *p, size_t n) {
  if (!p) return alloc(n, 16);
  Block *b = owned(p, "realloc");
  if (!b) return __real_realloc(p, n);
  if (!n) {
    release(b);
    return 0;
  }
  void *q = alloc(n, 16);
  if (!q) return 0;
  memcpy(q, p, b->size < n ? b->size : n);
  release(b);
  return q;
}

void *__wrap_reallocarray(void *p, size_t n, size_t m) {
  if (m && n > SIZE_MAX / m) return 0;
  return __wrap_realloc(p, n * m);
}

void *__wrap_aligned_alloc(size_t align, size_t n) {
  return align && !(align & (align - 1)) ? alloc(n, align) : 0;
}

int __wrap_posix_memalign(void **r, size_t align, size_t n) {
  if (!align || align & (align - 1) || align % sizeof(void *)) return 22; // EINVAL
  void *p = alloc(n, align);
  if (!p) return 12; // ENOMEM
  *r = p;
  return 0;
}

size_t __wrap_malloc_usable_size(void *p) {
  Block *b = p ? block((uintptr_t)p) : 0;
  return b ? b->size : p ? __real_malloc_usable_size(p) : 0;
}

// --- the leaks, as LSan's: the blocks that nothing reaches

// the loader's: __sc_rt_data says where the i-th range of the modules' data
// written since they were loaded is, end << 32 | start, and 0 after the
// last (their constants would look like pointers to the heap, which comes
// after them); __sc_rt_leak tells it of lost blocks allocated at one stack
__attribute__((weak)) uint64_t __sc_rt_data(uint32_t i);
__attribute__((weak)) void __sc_rt_leak(uint32_t allocated, uint32_t bytes, uint32_t blocks);

// the live blocks by address, which of them are reached, and those reached
// whose words are still to be scanned
typedef struct {
  Block **v;
  uint8_t *seen;
  uint32_t *work;
  size_t n, top;
} Reach;

static int by_address(const void *x, const void *y) {
  uintptr_t a = (uintptr_t)*(Block *const *)x, b = (uintptr_t)*(Block *const *)y;
  return (a > b) - (a < b);
}

static int by_stack(const void *x, const void *y) {
  uint32_t a = (*(Block *const *)x)->allocated, b = (*(Block *const *)y)->allocated;
  return (a > b) - (a < b);
}

// reaches the blocks the words in [beg, end) point into
static void scan(Reach *r, uintptr_t beg, uintptr_t end) {
  for (uintptr_t p = (beg + 3) & ~(uintptr_t)3; p + 4 <= end; p += 4) {
    uintptr_t a = *(const uint32_t *)p;
    if (a < SHADOW + (512u << 20)) continue;
    size_t lo = 0, hi = r->n; // after the last block whose header is before a
    while (lo < hi) {
      size_t m = (lo + hi) / 2;
      if ((uintptr_t)r->v[m] < a) lo = m + 1;
      else hi = m;
    }
    if (!lo) continue;
    Block *b = r->v[--lo];
    uintptr_t user = (uintptr_t)b + sizeof(Block);
    if (a >= user && a - user < (b->size ? b->size : 1) && !r->seen[lo]) {
      r->seen[lo] = 1;
      r->work[r->top++] = lo;
    }
  }
}

// The live blocks that nothing reaches: neither the stack above sp, nor the
// modules' data, nor a block reached. It tells the loader of them, a
// group for each stack where they were allocated, and says how many.
uint32_t __sc_rt_leaks(uintptr_t sp) {
  size_t n = 0;
  for (Block *b = live; b; b = b->next) n++;
  if (!n || !__sc_rt_data) return 0;
  Reach r = {.v = __real_malloc(n * sizeof(Block *)), .seen = __real_malloc(n), .work = __real_malloc(n * 4), .n = n};
  size_t k = 0;
  if (r.v && r.seen && r.work) {
    n = 0;
    for (Block *b = live; b; b = b->next) r.v[n++] = b;
    qsort(r.v, n, sizeof *r.v, by_address);
    memset(r.seen, 0, n);
    scan(&r, sp, (uintptr_t)&__stack_high);
    uint64_t d;
    for (uint32_t i = 0; (d = __sc_rt_data(i)); i++) scan(&r, (uint32_t)d, d >> 32);
    while (r.top) {
      Block *b = r.v[r.work[--r.top]];
      uintptr_t user = (uintptr_t)b + sizeof(Block);
      scan(&r, user, user + b->size);
    }
    for (size_t i = 0; i < n; i++)
      if (!r.seen[i]) r.v[k++] = r.v[i];
    qsort(r.v, k, sizeof *r.v, by_stack);
    for (size_t i = 0, j; i < k; i = j) {
      uint32_t bytes = 0;
      for (j = i; j < k && r.v[j]->allocated == r.v[i]->allocated; j++) bytes += r.v[j]->size;
      if (__sc_rt_leak) __sc_rt_leak(r.v[i]->allocated, bytes, j - i);
    }
  }
  __real_free(r.v), __real_free(r.seen), __real_free(r.work);
  return k;
}

// libc's getdelim reallocates the line with its own realloc: it reads into a
// buffer of its own, which goes to the program's
ssize_t __real_getdelim(char **, size_t *, int, FILE *);

ssize_t __wrap_getdelim(char **line, size_t *n, int delim, FILE *f) {
  char *buf = 0;
  size_t cap = 0;
  ssize_t r = __real_getdelim(&buf, &cap, delim, f);
  if (r >= 0) {
    if (*line && *n >= (size_t)r + 1) {
      check((uintptr_t)*line, r + 1, 1, "getline");
    } else {
      char *p = __wrap_realloc(*line, r + 1);
      if (!p) r = -1;
      else *line = p, *n = r + 1;
    }
    if (r >= 0) memcpy(*line, buf, r + 1);
  }
  __wrap_free(buf);
  return r;
}

ssize_t __wrap_getline(char **line, size_t *n, FILE *f) { return __wrap_getdelim(line, n, '\n', f); }

// --- libc's functions that write the program's memory, checked

char *__real_strcpy(char *, const char *);
char *__real_strncpy(char *, const char *, size_t);
char *__real_strcat(char *, const char *);
char *__real_strncat(char *, const char *, size_t);
char *__real_fgets(char *, int, FILE *);
size_t __real_fread(void *, size_t, size_t, FILE *);
ssize_t __real_read(int, void *, size_t);

char *__wrap_strcpy(char *d, const char *s) {
  check_str(s, 0, "strcpy");
  check((uintptr_t)d, strlen(s) + 1, 1, "strcpy");
  return __real_strcpy(d, s);
}

char *__wrap_strncpy(char *d, const char *s, size_t n) {
  check((uintptr_t)s, strnlen(s, n) + (strnlen(s, n) < n), 0, "strncpy");
  check((uintptr_t)d, n, 1, "strncpy");
  return __real_strncpy(d, s, n);
}

char *__wrap_strcat(char *d, const char *s) {
  check_str(s, 0, "strcat");
  check_str(d, 0, "strcat");
  check((uintptr_t)d + strlen(d), strlen(s) + 1, 1, "strcat");
  return __real_strcat(d, s);
}

char *__wrap_strncat(char *d, const char *s, size_t n) {
  size_t m = strnlen(s, n);
  check((uintptr_t)s, m + (m < n), 0, "strncat");
  check_str(d, 0, "strncat");
  check((uintptr_t)d + strlen(d), m + 1, 1, "strncat");
  return __real_strncat(d, s, n);
}

static int checked_vsprintf(char *d, const char *fmt, va_list ap) {
  va_list aq;
  va_copy(aq, ap);
  int n = vsnprintf(0, 0, fmt, aq);
  va_end(aq);
  if (n >= 0) check((uintptr_t)d, n + 1, 1, "sprintf");
  return vsprintf(d, fmt, ap);
}

static int checked_vsnprintf(char *d, size_t cap, const char *fmt, va_list ap) {
  va_list aq;
  va_copy(aq, ap);
  int n = vsnprintf(0, 0, fmt, aq);
  va_end(aq);
  if (n >= 0 && cap) check((uintptr_t)d, (size_t)n + 1 < cap ? (size_t)n + 1 : cap, 1, "snprintf");
  return vsnprintf(d, cap, fmt, ap);
}

int __wrap_sprintf(char *d, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int n = checked_vsprintf(d, fmt, ap);
  va_end(ap);
  return n;
}

int __wrap_snprintf(char *d, size_t cap, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int n = checked_vsnprintf(d, cap, fmt, ap);
  va_end(ap);
  return n;
}

// the room the program says it has, not only what this input fills
char *__wrap_fgets(char *s, int n, FILE *f) {
  if (n > 0) check_room((uintptr_t)s, n, "fgets");
  return __real_fgets(s, n, f);
}

size_t __wrap_fread(void *p, size_t size, size_t n, FILE *f) {
  if (!size || n <= SIZE_MAX / size) check_room((uintptr_t)p, size * n, "fread");
  return __real_fread(p, size, n, f);
}

ssize_t __wrap_read(int fd, void *p, size_t n) {
  check_room((uintptr_t)p, n, "read");
  return __real_read(fd, p, n);
}

// --- scanf: libc writes each conversion through its pointer. What the format
// says the size of (%d, %9s, %c) is checked before the call, as the room of
// fgets; a %s or %[ without a width, after it, for the conversions it made
int __real_vfscanf(FILE *, const char *, va_list);
int __real_vsscanf(const char *, const char *, va_list);

// the pointers of fmt's conversions: done < 0, before the call, those of a
// known size; else, after it, the strings without a width among the first
// done conversions. A positional argument (%1$d) stops the check
static void scan_args(const char *fn, const char *fmt, va_list ap, int done) {
  for (const char *f = fmt; *f; f++) {
    if (*f != '%') continue;
    if (*++f == '%') continue;
    int skip = *f == '*';
    f += skip;
    size_t width = 0;
    while (*f >= '0' && *f <= '9') width = width * 10 + (size_t)(*f++ - '0');
    if (*f == '$') return;
    int alloc = *f == 'm';
    f += alloc;
    int len = 0; // h -1, hh -2, l 1, ll 2; j as ll; z and t as int; L as ll
    if (*f == 'h') len = f[1] == 'h' ? (f += 2, -2) : (f++, -1);
    else if (*f == 'l') len = f[1] == 'l' ? (f += 2, 2) : (f++, 1);
    else if (*f == 'j' || *f == 'L' || *f == 'q') len = 2, f++;
    else if (*f == 'z' || *f == 't') f++;
    char c = *f;
    if (!c) return;
    if (c == '[') { // the set: a ] first is in it
      f += f[1] == '^';
      f += f[1] == ']';
      while (f[1] && f[1] != ']') f++;
      if (!*++f) return;
    }
    if (skip) continue;
    void *p = va_arg(ap, void *);
    uintptr_t a = (uintptr_t)p, n;
    int str = c == 's' || c == '[' || c == 'S' || c == 'c' || c == 'C';
    size_t esz = len == 1 || c == 'S' || c == 'C' ? sizeof(wchar_t) : 1;
    if (done >= 0) {
      if (c != 'n' && done-- == 0) return;
      if ((c == 's' || c == '[' || c == 'S') && !width && !alloc)
        check(a, esz == 1 ? strlen(p) + 1 : (wcslen(p) + 1) * esz, 1, fn);
      continue;
    }
    if (alloc && str) n = sizeof(char *);
    else if (c == 'c' || c == 'C') n = (width ? width : 1) * esz;
    else if (str) {
      if (width) check_room(a, (width + 1) * esz, fn);
      continue;
    } else if (strchr("aAeEfFgG", c)) n = len == 2 ? sizeof(long double) : len == 1 ? sizeof(double) : sizeof(float);
    else if (c == 'p') n = sizeof(void *);
    else n = len == 2 ? sizeof(long long) : len == 1 ? sizeof(long) : len == -1 ? sizeof(short) : len == -2 ? 1 : sizeof(int);
    check(a, n, 1, fn);
  }
}

static int checked_vfscanf(const char *fn, FILE *s, const char *str, const char *fmt, va_list ap) {
  va_list before, after;
  if (str) check_str(str, 0, fn);
  va_copy(before, ap);
  va_copy(after, ap);
  scan_args(fn, fmt, before, -1);
  va_end(before);
  int r = str ? __real_vsscanf(str, fmt, ap) : __real_vfscanf(s, fmt, ap);
  if (r > 0) scan_args(fn, fmt, after, r);
  va_end(after);
  return r;
}

int __wrap_vfscanf(FILE *s, const char *fmt, va_list ap) { return checked_vfscanf("fscanf", s, 0, fmt, ap); }
int __wrap_vscanf(const char *fmt, va_list ap) { return checked_vfscanf("scanf", stdin, 0, fmt, ap); }
int __wrap_vsscanf(const char *str, const char *fmt, va_list ap) { return checked_vfscanf("sscanf", 0, str, fmt, ap); }

#define SCANF(name, ...)                                                                                               \
  {                                                                                                                    \
    va_list ap;                                                                                                        \
    va_start(ap, fmt);                                                                                                 \
    int r = checked_vfscanf(name, __VA_ARGS__, fmt, ap);                                                               \
    va_end(ap);                                                                                                        \
    return r;                                                                                                          \
  }
int __wrap_scanf(const char *fmt, ...) SCANF("scanf", stdin, 0)
int __wrap_fscanf(FILE *s, const char *fmt, ...) SCANF("fscanf", s, 0)
int __wrap_sscanf(const char *str, const char *fmt, ...) SCANF("sscanf", 0, str)

// --- C++: a throw from libc++ (at, stoi) leaves frames without the pass's
// __asan_handle_no_return
void __real___cxa_throw(void *, void *, void (*)(void *)) __attribute__((weak, noreturn));
void __real___cxa_rethrow(void) __attribute__((weak, noreturn));

void __wrap___cxa_throw(void *e, void *type, void (*dtor)(void *)) {
  __asan_handle_no_return();
  __real___cxa_throw(e, type, dtor);
}

void __wrap___cxa_rethrow(void) {
  __asan_handle_no_return();
  __real___cxa_rethrow();
}
