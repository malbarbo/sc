/* entry 1: a jmp_buf and a function that jumps back to it */
#include <setjmp.h>
jmp_buf env;
void fail(int code) { longjmp(env, code); }
