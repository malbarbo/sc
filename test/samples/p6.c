/* setjmp/longjmp: an error that returns from deep calls, and a retry loop */
#include <setjmp.h>
#include <stdio.h>
#include <string.h>
static jmp_buf on_error;
static const char *message;
static int parse_digit(char c) {
  if (c < '0' || c > '9') {
    message = "não é dígito";
    longjmp(on_error, c);
  }
  return c - '0';
}
static int parse(const char *s) {
  int n = 0;
  for (; *s; s++) n = n * 10 + parse_digit(*s);
  return n;
}
int main(void) {
  const char *inputs[] = {"123", "4x5", "", "99"};
  for (int i = 0; i < 4; i++) {
    volatile int tries = 0;
    int c = setjmp(on_error);
    if (c == 0) {
      printf("%s -> %d\n", inputs[i], parse(inputs[i]));
    } else {
      tries++;
      printf("%s: erro '%c' (%s), tentativas %d\n", inputs[i], c, message, tries);
    }
  }
  jmp_buf a;
  volatile int count = 0;
  if (setjmp(a) < 3) {
    count++;
    longjmp(a, count);
  }
  printf("count %d\n", count);
  return 0;
}
