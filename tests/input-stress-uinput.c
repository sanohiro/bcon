// Bounded end-to-end keyboard stress test. Run inside bcon via sudo.
#include <fcntl.h>
#include <linux/uinput.h>
#include <poll.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

static const struct {
    int code;
    const char *name;
    const char *expect;
} keys[] = {
    {KEY_A, "a", "a"}, {KEY_Z, "z", "z"}, {KEY_SPACE, "space", " "},
    {KEY_UP, "Up", "\033[A"}, {KEY_DOWN, "Down", "\033[B"},
    {KEY_LEFT, "Left", "\033[D"}, {KEY_RIGHT, "Right", "\033[C"},
    {KEY_KP8, "KP8/Up", "\033[A"}, {KEY_KP2, "KP2/Down", "\033[B"},
    {KEY_KP4, "KP4/Left", "\033[D"}, {KEY_KP6, "KP6/Right", "\033[C"},
    {KEY_KP7, "KP7/Home", "\033[H"}, {KEY_KP1, "KP1/End", "\033[F"},
    {KEY_KP9, "KP9/PageUp", "\033[5~"}, {KEY_KP3, "KP3/PageDown", "\033[6~"},
    {KEY_KP0, "KP0/Insert", "\033[2~"}, {KEY_KPDOT, "KP./Delete", "\033[3~"},
};

static volatile sig_atomic_t stop_requested;

static void on_signal(int number) { (void)number; stop_requested = 1; }

static double seconds_now(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return now.tv_sec + now.tv_nsec / 1000000000.0;
}

static int emit(int fd, unsigned short type, unsigned short code, int value) {
    struct input_event event = {.type = type, .code = code, .value = value};
    return write(fd, &event, sizeof(event)) == sizeof(event) ? 0 : -1;
}

static int press(int fd, int code) {
    if (emit(fd, EV_KEY, code, 1) || emit(fd, EV_SYN, SYN_REPORT, 0)) return -1;
    usleep(25000);
    if (emit(fd, EV_KEY, code, 0) || emit(fd, EV_SYN, SYN_REPORT, 0)) return -1;
    return 0;
}

static int receive(char *buffer, size_t size, int timeout_ms) {
    int used = 0;
    while (used + 1 < (int)size && !stop_requested) {
        struct pollfd pfd = {.fd = STDIN_FILENO, .events = POLLIN};
        if (poll(&pfd, 1, timeout_ms) <= 0) break;
        ssize_t count = read(STDIN_FILENO, buffer + used, size - used - 1);
        if (count <= 0) break;
        used += (int)count;
        timeout_ms = 15;
    }
    buffer[used] = '\0';
    if (memchr(buffer, 'q', used) || memchr(buffer, 3, used)) stop_requested = 1;
    return used;
}

static void show_status(double elapsed, int minutes, unsigned long total, unsigned long errors,
                        const char *last_key) {
    printf("\033[2J\033[HKeyboard stress: %.0f / %d seconds\r\n", elapsed, minutes * 60);
    printf("Keys: %lu   mismatches: %lu   last: %s\r\n", total, errors, last_key);
    printf("Sending letters, arrows and keypad keys. q or Ctrl+C: stop.\r\n\r\n");
    for (int row = 0; row < 10; ++row) {
        for (int col = 0; col < 25; ++col) {
            int red = (total + row * 11 + col * 7) % 256;
            int blue = (total * 2 + row * 5 + col * 13) % 256;
            printf("\033[48;2;%d;50;%dm  ", red, blue);
        }
        printf("\033[0m\r\n");
    }
    fflush(stdout);
}

int main(int argc, char **argv) {
    char *end = NULL;
    long minutes = argc > 1 ? strtol(argv[1], &end, 10) : 10;
    if ((argc > 1 && (!end || *end)) || minutes < 1 || minutes > 60) {
        fprintf(stderr, "Usage: binputstress [1..60 minutes]\n");
        return 2;
    }
    int fd = open("/dev/uinput", O_WRONLY | O_NONBLOCK);
    if (fd < 0) { perror("/dev/uinput (sudo required)"); return 2; }
    if (ioctl(fd, UI_SET_EVBIT, EV_KEY) < 0 || ioctl(fd, UI_SET_EVBIT, EV_SYN) < 0) {
        perror("UI_SET_EVBIT"); close(fd); return 2;
    }
    for (size_t i = 0; i < sizeof(keys) / sizeof(keys[0]); ++i) {
        if (ioctl(fd, UI_SET_KEYBIT, keys[i].code) < 0) {
            perror("UI_SET_KEYBIT"); close(fd); return 2;
        }
    }
    ioctl(fd, UI_SET_KEYBIT, KEY_NUMLOCK);
    struct uinput_setup setup = {.id = {.bustype = BUS_USB, .vendor = 0x1234, .product = 0x5679}};
    snprintf(setup.name, sizeof(setup.name), "bcon temporary keyboard stress");
    if (ioctl(fd, UI_DEV_SETUP, &setup) < 0 || ioctl(fd, UI_DEV_CREATE) < 0) {
        perror("UI_DEV_SETUP/CREATE"); close(fd); return 2;
    }
    char log_name[] = "/var/tmp/bcon-input-stress-XXXXXX";
    int log_fd = mkstemp(log_name);
    if (log_fd >= 0) {
        fchmod(log_fd, 0644);
        dprintf(log_fd, "START duration=%ldm\n", minutes);
        fsync(log_fd);
    }
    printf("Virtual keyboard ready. Log: %s\n", log_name);
    fflush(stdout);
    sleep(2);

    struct termios old, raw;
    if (tcgetattr(STDIN_FILENO, &old) < 0) {
        perror("tcgetattr"); ioctl(fd, UI_DEV_DESTROY); close(fd); return 2;
    }
    raw = old;
    cfmakeraw(&raw);
    tcsetattr(STDIN_FILENO, TCSANOW, &raw);
    signal(SIGTERM, on_signal);
    signal(SIGINT, on_signal);
    printf("\033[?1049h\033[?25l");
    fflush(stdout);
    char got[128];
    receive(got, sizeof(got), 10);
    press(fd, KEY_KP8);
    int length = receive(got, sizeof(got), 500);
    bool changed_numlock = false;
    if (length == 1 && got[0] == '8') {
        press(fd, KEY_NUMLOCK);
        changed_numlock = true;
        receive(got, sizeof(got), 100);
        press(fd, KEY_KP8);
        length = receive(got, sizeof(got), 500);
    }
    unsigned long total = 0, errors = 0;
    if (length != 3 || memcmp(got, "\033[A", 3) != 0) {
        errors++;
        stop_requested = 1;
        if (log_fd >= 0) dprintf(log_fd, "ERROR virtual keyboard not reaching bcon\n");
    }
    double start = seconds_now();
    const char *last_key = "ready";
    while (!stop_requested && seconds_now() - start < minutes * 60.0) {
        size_t index = total % (sizeof(keys) / sizeof(keys[0]));
        receive(got, sizeof(got), 1);
        if (press(fd, keys[index].code) < 0) {
            errors++;
            break;
        }
        length = receive(got, sizeof(got), 500);
        if (stop_requested) break;
        total++;
        last_key = keys[index].name;
        size_t expected = strlen(keys[index].expect);
        if ((size_t)length != expected || memcmp(got, keys[index].expect, expected) != 0) {
            errors++;
            if (log_fd >= 0) {
                dprintf(log_fd, "MISMATCH key=%s length=%d bytes=", last_key, length);
                for (int i = 0; i < length; ++i) dprintf(log_fd, "%02X", (unsigned char)got[i]);
                dprintf(log_fd, "\n");
            }
        }
        if (total % 50 == 0) {
            show_status(seconds_now() - start, (int)minutes, total, errors, last_key);
            if (log_fd >= 0) {
                dprintf(log_fd, "PROGRESS elapsed=%.1fs keys=%lu errors=%lu\n",
                        seconds_now() - start, total, errors);
                fsync(log_fd);
            }
        }
    }
    double elapsed = seconds_now() - start;
    if (changed_numlock) press(fd, KEY_NUMLOCK);
    printf("\033[0m\033[?25h\033[?1049l");
    fflush(stdout);
    tcsetattr(STDIN_FILENO, TCSADRAIN, &old);
    ioctl(fd, UI_DEV_DESTROY);
    close(fd);
    if (log_fd >= 0) {
        dprintf(log_fd, "%s elapsed=%.1fs keys=%lu errors=%lu\n",
                stop_requested ? "STOPPED" : "COMPLETE", elapsed, total, errors);
        fsync(log_fd);
        close(log_fd);
    }
    printf("Keyboard stress: %.1fs, %lu keys, %lu mismatches. Log: %s\n",
           elapsed, total, errors, log_name);
    return errors ? 1 : 0;
}
