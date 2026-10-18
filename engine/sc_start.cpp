// The command entry of sc.wasm, a reactor (PLANO-REPL.md §5.2): crt1-reactor's
// _initialize runs the constructors, which `wasmtime run` calls before
// _start. A host that drives the session (sc.h) calls _initialize once and
// then the exports, never _start.
#include <stdlib.h>

extern "C" int __main_void(void);

extern "C" __attribute__((export_name("_start"))) void _start(void) { exit(__main_void()); }
