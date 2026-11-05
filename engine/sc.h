/* The engine's API (PLANO-REPL.md §2): sc.wasm's exports. Only scalars and
   pointers. The Rust side takes the types and declarations from here
   through bindgen (src/engine).

   A result belongs to the engine and is valid until the next call on the
   same session. The engine is not thread safe: one thread uses it. A fatal
   error in the compiler ends the module (a trap or an exit): with
   SC_SNAPSHOT, sc_session_recover puts the session back as before the cell
   (ANALISE.md §65). */
#ifndef SC_H
#define SC_H

#ifdef __cplusplus
extern "C" {
#endif

enum sc_status {
  SC_OK = 0,    /* done */
  SC_ERROR = 1, /* an error in the student's code. A failed cell leaves part of
                   itself in the session (§3.3); with SC_SNAPSHOT the copy
                   taken before it puts the session back */
  SC_LOST = 2,  /* after a fatal error, a failed link or an undo: the session must be
                   recreated and the log replayed (§3.6) */
  SC_INCOMPLETE = 3, /* SC_CELL_WHOLE: the cell stops short, as a failed cell
                        (SC_ERROR) without its diagnostics */
};

enum sc_part {
  SC_DIAGS = 0, /* the diagnostics of the call, as text */
  SC_SO = 1,    /* the cell as a wasm shared object */
  SC_SYMS = 2,  /* the names the cell exports, one per line, to check a
                   replay; SC_CELL_TYPE: the type */
};

enum sc_cell_mode {
  SC_CELL_SO = 0,     /* compile and link the cell: SC_SO and SC_SYMS */
  SC_CELL_REPLAY = 1, /* parse and generate IR only, as in a replay: SC_SYMS */
  SC_CELL_WHOLE = 2,  /* SC_CELL_SO, or SC_INCOMPLETE if the cell stops short:
                         its first error is at its end, where the parser wanted
                         more (a declaration without its `;`, a function's
                         header without its body, an `if` without its
                         statement). The REPL asks so on Enter */
  SC_CELL_TYPE = 3,   /* the cell is an expression without its `;`: SC_OK with
                         its type in SC_SYMS, as a shown value writes it
                         (below). Nothing is generated, but as a failed cell
                         (SC_ERROR) it leaves part of itself in the session */
};

/* A cell that ends in an expression without its `;` shows its value, if the
   value is a number, a character, a bool or a pointer: the expression
   becomes a call to
     void __sc_show_int(const char *type, int kind, long long value);
     void __sc_show_float(const char *type, int kind, double value);
   which the host that loads the cells defines. type is the expression's type
   as clang writes it: a typedef at namespace scope (size_t, std::string)
   keeps its name, the rest is canonical; kind is one of these. A string is the address of its
   first char. Any other value is not shown. */
enum sc_show {
  SC_SHOW_SIGNED = 0,
  SC_SHOW_UNSIGNED = 1,
  SC_SHOW_BOOL = 2,
  SC_SHOW_CHAR = 3,
  SC_SHOW_POINTER = 4,
  SC_SHOW_STRING = 5,  /* a char * or a char array */
  SC_SHOW_NULLPTR = 6, /* nullptr_t */
  SC_SHOW_DOUBLE = 7,  /* __sc_show_float: double, long double */
  SC_SHOW_FLOAT = 8,   /* __sc_show_float: float */
};

typedef struct sc_session sc_session;

/* A session. argv are cc1 flags without an input file, -x c or -x c++ (C by
   default), then, after "--", the wasm-ld arguments that link a cell (-L,
   the libraries, the builtins). *out is always set, also when the session
   could not start (SC_ERROR, with SC_DIAGS): free it with sc_session_free. */
int sc_session_new(int argc, const char *const *argv, sc_session **out);
void sc_session_free(sc_session *s);
/* Compiles src[0, len) on top of the cells before (mode: enum sc_cell_mode). */
int sc_session_cell(sc_session *s, const char *src, unsigned len, int mode);
/* Undoes the last accepted cell. Its code is already generated, so this is
   SC_LOST: the host replays the log without it (§3.3). SC_ERROR if no cell
   was accepted. */
int sc_session_undo(sc_session *s);
/* sc.wasm with SC_SNAPSHOT (sc_api.cpp): after a trap or an exit in
   sc_session_cell, the session as it was before the cell, from the copy the
   cell took, and SC_ERROR; SC_LOST if there is none (the host replays the
   log). The host calls it first, before any other export. */
int sc_session_recover(sc_session *s);
/* sc_session_recover for a caller whose frames are still there (the
   machine inside sc.wasm, which called sc_session_cell through its host):
   the memory back, the stack pointer as it is. */
int sc_session_restore(sc_session *s);
/* sc.wasm: whether sc_session_cell takes the copy, as SC_SNAPSHOT does. */
void sc_session_snapshot(int on);
/* The brackets src[0, len) leaves open, by the session's lexer, one more if
   it ends inside a comment or with a `\` that joins the next line: how far
   in the REPL starts the line Enter opens. It does not touch the session. */
int sc_session_depth(sc_session *s, const char *src, unsigned len);
/* A part of the result of the last call (enum sc_part). */
const unsigned char *sc_result_ptr(sc_session *s, int part);
unsigned sc_result_len(sc_session *s, int part);

/* The files a session reads in place of the disk: those under root (the
   pack inside sc.wasm, PLANO-REPL.md §5.4), for its clang and its wasm-ld.
   The rest are on the disk, and so is all without them and in the one-shot
   engine. sc.wasm does not export it: its host turns its own pack on with
   sc_pack (sc_pack.cpp). A path
   asked for is absolute and without "." or "..", with '/' or the system's
   separator. What read and list return is valid until the process ends. */
struct sc_files {
  const char *root; /* "/sc" */
  /* 0: no such path; 1: a file of *size bytes; 2: a directory */
  int (*status)(const char *path, unsigned long long *size);
  /* a file's *size bytes, with a NUL after them; NULL if it is not a file */
  const char *(*read)(const char *path, unsigned long long *size);
  /* the names in a directory, each followed by a NUL, *size bytes in all;
     NULL if it is not a directory */
  const char *(*list)(const char *path, unsigned long long *size);
};
void sc_set_files(const struct sc_files *files);

/* The one-shot engine: sc -cc1 ..., sc ld ... (argv[0] is the program). */
int sc_main(int argc, const char **argv);

#ifdef __cplusplus
}
#endif

#endif
