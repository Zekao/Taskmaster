#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

int main(void) {
  int fd = open("test.txt", O_CREAT | O_RDWR, 0777);
  if (fd < 0)
    return 1;
  return close(fd), 0;
}
