// an exception that leaves a noexcept function ends the program with
// std::terminate, after the handler set by std::set_terminate
#include <cstdio>
#include <cstdlib>
#include <exception>
#include <stdexcept>
void inner() { throw std::runtime_error("escapou"); }
void wrapper() noexcept { inner(); }
int main() {
  std::set_terminate([] {
    std::puts("terminate");
    std::fflush(stdout);
    std::abort();
  });
  std::puts("antes");
  wrapper();
  std::puts("depois?");
}
