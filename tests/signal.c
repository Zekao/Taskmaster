/*
    This function is used to test the signals

    It will block the signal SIGINT in order to see what will happen when we try
   to kill the process with the signal SIGINT, it should do a force kill witha a
   SIGKILL after the exit timeout
*/

#include <signal.h>
#include <unistd.h>

int main(void) {
  signal(SIGINT, SIG_IGN);
  while (1) {
  }
}
