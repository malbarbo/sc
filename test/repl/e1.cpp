// entry 1: a function that throws and a class hierarchy for errors
#include <stdexcept>
#include <string>
struct Problem : std::runtime_error { using std::runtime_error::runtime_error; };
int check(int x) {
  if (x < 0) throw Problem("negativo: " + std::to_string(x));
  return x;
}
