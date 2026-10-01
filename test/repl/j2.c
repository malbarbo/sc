/* entry 2: setjmp here, longjmp in entry 1 */
#include <setjmp.h>
#include <stdio.h>
extern jmp_buf env;
void fail(int code);
int main(void) {
  int r = setjmp(env);
  printf("setjmp %d\n", r);
  if (r < 2) fail(r + 1);
  return 0;
}
