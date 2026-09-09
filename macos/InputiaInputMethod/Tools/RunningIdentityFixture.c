#include <unistd.h>
#ifndef FIXTURE_VERSION
#define FIXTURE_VERSION 1
#endif
volatile int fixture_version = FIXTURE_VERSION;
int main(void) {
  while (fixture_version > 0) sleep(1);
  return 0;
}
