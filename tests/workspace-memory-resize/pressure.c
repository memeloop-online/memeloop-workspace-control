#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>

int main(void) {
    const size_t anonymous_bytes = 196 * 1024 * 1024;
    const off_t file_bytes = 192 * 1024 * 1024;
    const size_t block = 64 * 1024;
    volatile unsigned char *memory = mmap(NULL, anonymous_bytes, PROT_READ | PROT_WRITE,
                                          MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (memory == MAP_FAILED) {
        perror("mmap");
        return 1;
    }
    for (size_t i = 0; i < anonymous_bytes; i += 4096) {
        memory[i] = 1;
    }
    int file = open("/tmp/pressure-input", O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (file < 0) {
        perror("open fixture file");
        return 1;
    }
    unsigned char data[64 * 1024] = {1};
    for (off_t i = 0; i < file_bytes; i += (off_t)block) {
        if (pwrite(file, data, block, i) != (ssize_t)block) {
            perror("write fixture file");
            return 1;
        }
    }
    if (fdatasync(file) != 0) {
        perror("flush fixture file");
        return 1;
    }
    while (access("/tmp/pressure-stop", F_OK) != 0) {
        for (off_t i = 0; i < file_bytes; i += (off_t)block) {
            if (pread(file, data, block, i) != (ssize_t)block) {
                perror("read fixture file");
                return 1;
            }
            memory[(size_t)i] ^= data[0];
        }
    }
    return close(file) != 0;
}
