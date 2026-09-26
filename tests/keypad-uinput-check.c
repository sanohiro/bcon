// Inject a temporary virtual keypad and verify bcon's PTY key sequences.
// Build: cc -O2 -Wall -Wextra -o /tmp/bcon-keypad-check tests/keypad-uinput-check.c
// Run inside bcon: sudo /tmp/bcon-keypad-check
#include <errno.h>
#include <fcntl.h>
#include <linux/uinput.h>
#include <poll.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

static const struct {
    int code;
    const char *name;
    const char *expected;
} cases[] = {
    {KEY_KP8, "KP8 / Up", "\033[A"},
    {KEY_KP2, "KP2 / Down", "\033[B"},
    {KEY_KP6, "KP6 / Right", "\033[C"},
    {KEY_KP4, "KP4 / Left", "\033[D"},
    {KEY_KP7, "KP7 / Home", "\033[H"},
    {KEY_KP1, "KP1 / End", "\033[F"},
    {KEY_KP9, "KP9 / PageUp", "\033[5~"},
    {KEY_KP3, "KP3 / PageDown", "\033[6~"},
    {KEY_KP0, "KP0 / Insert", "\033[2~"},
    {KEY_KPDOT, "KP. / Delete", "\033[3~"},
};

static int send_event(int fd, unsigned short type, unsigned short code, int value) {
    struct input_event event = {.type = type, .code = code, .value = value};
    return write(fd, &event, sizeof(event)) == sizeof(event) ? 0 : -1;
}

static int send_key(int fd, int code) {
    if (send_event(fd, EV_KEY, code, 1) || send_event(fd, EV_SYN, SYN_REPORT, 0)) return -1;
    usleep(30000);
    if (send_event(fd, EV_KEY, code, 0) || send_event(fd, EV_SYN, SYN_REPORT, 0)) return -1;
    return 0;
}

static int collect(char *out, size_t capacity, int timeout_ms) {
    size_t used = 0;
    while (used + 1 < capacity) {
        struct pollfd pfd = {.fd = STDIN_FILENO, .events = POLLIN};
        int ready = poll(&pfd, 1, timeout_ms);
        if (ready <= 0) break;
        ssize_t count = read(STDIN_FILENO, out + used, capacity - used - 1);
        if (count <= 0) break;
        used += (size_t)count;
        timeout_ms = 40;
    }
    out[used] = '\0';
    return (int)used;
}

static void show_bytes(const char *bytes, int length) {
    for (int i = 0; i < length; ++i) printf(" %02X", (unsigned char)bytes[i]);
}

int main(void) {
    int fd = open("/dev/uinput", O_WRONLY | O_NONBLOCK);
    if (fd < 0) {
        perror("open /dev/uinput (sudo required)");
        return 2;
    }
    int keys[] = {KEY_A, KEY_Z, KEY_SPACE, KEY_ENTER, KEY_NUMLOCK,
                  KEY_KP0, KEY_KP1, KEY_KP2, KEY_KP3, KEY_KP4, KEY_KP5,
                  KEY_KP6, KEY_KP7, KEY_KP8, KEY_KP9, KEY_KPDOT};
    if (ioctl(fd, UI_SET_EVBIT, EV_KEY) < 0 || ioctl(fd, UI_SET_EVBIT, EV_SYN) < 0) {
        perror("UI_SET_EVBIT");
        close(fd);
        return 2;
    }
    for (size_t i = 0; i < sizeof(keys) / sizeof(keys[0]); ++i) {
        if (ioctl(fd, UI_SET_KEYBIT, keys[i]) < 0) {
            perror("UI_SET_KEYBIT");
            close(fd);
            return 2;
        }
    }
    struct uinput_setup setup = {.id = {.bustype = BUS_USB, .vendor = 0x1234, .product = 0x5678}};
    snprintf(setup.name, sizeof(setup.name), "bcon temporary keypad test");
    if (ioctl(fd, UI_DEV_SETUP, &setup) < 0 || ioctl(fd, UI_DEV_CREATE) < 0) {
        perror("UI_DEV_SETUP/CREATE");
        close(fd);
        return 2;
    }
    printf("Virtual keypad ready; waiting for libinput...\r\n");
    fflush(stdout);
    sleep(2);

    struct termios old, raw;
    if (tcgetattr(STDIN_FILENO, &old) < 0) {
        perror("tcgetattr");
        ioctl(fd, UI_DEV_DESTROY);
        close(fd);
        return 2;
    }
    raw = old;
    cfmakeraw(&raw);
    tcsetattr(STDIN_FILENO, TCSANOW, &raw);
    char received[128];
    collect(received, sizeof(received), 10);

    // A fresh XKB state starts with NumLock off. Toggle once if it was on.
    send_key(fd, KEY_KP8);
    int length = collect(received, sizeof(received), 600);
    bool initially_numlock_on = length == 1 && received[0] == '8';
    if (initially_numlock_on) {
        send_key(fd, KEY_NUMLOCK);
        collect(received, sizeof(received), 100);
        send_key(fd, KEY_KP8);
        length = collect(received, sizeof(received), 600);
    }
    int passed = 0;
    int failed = 0;
    if (length == 3 && memcmp(received, "\033[A", 3) == 0) {
        printf("PASS KP8 / Up (NumLock off)\r\n");
        passed++;
    } else {
        printf("FAIL KP8 / Up: got");
        show_bytes(received, length);
        printf("\r\n");
        failed++;
    }
    for (size_t i = 1; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        collect(received, sizeof(received), 10);
        send_key(fd, cases[i].code);
        length = collect(received, sizeof(received), 600);
        size_t expected_length = strlen(cases[i].expected);
        if ((size_t)length == expected_length && memcmp(received, cases[i].expected, expected_length) == 0) {
            printf("PASS %s\r\n", cases[i].name);
            passed++;
        } else {
            printf("FAIL %s: got", cases[i].name);
            show_bytes(received, length);
            printf("\r\n");
            failed++;
        }
        fflush(stdout);
    }
    send_key(fd, KEY_NUMLOCK);
    collect(received, sizeof(received), 100);
    send_key(fd, KEY_KP8);
    length = collect(received, sizeof(received), 600);
    if (length == 1 && received[0] == '8') {
        printf("PASS KP8 / digit (NumLock on)\r\n");
        passed++;
    } else {
        printf("FAIL KP8 / digit: got");
        show_bytes(received, length);
        printf("\r\n");
        failed++;
    }
    if (!initially_numlock_on) send_key(fd, KEY_NUMLOCK); // Restore the original state.
    tcsetattr(STDIN_FILENO, TCSANOW, &old);
    ioctl(fd, UI_DEV_DESTROY);
    close(fd);
    printf("Result: %d passed, %d failed\n", passed, failed);
    return failed ? 1 : 0;
}
