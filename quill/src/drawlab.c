// drawlab: no-AI drawing experiment app for quill takeover.
// Blank paper + live pen ink + solid footstep sprites + delayed segment erase.
// Exit: power button, 5-finger tap, or SIGTERM.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <signal.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/time.h>
#include <poll.h>

extern int quill_init(void);
extern int quill_width(void);
extern int quill_height(void);
extern int quill_stride(void);
extern int quill_format(void);
extern unsigned char *quill_buffer(void);
extern unsigned long quill_swap(int x, int y, int w, int h, int mode, int full);
extern void quill_process_events(void);

#define EV_SYN 0
#define EV_KEY 1
#define EV_ABS 3
#define SYN_REPORT 0
#define ABS_X 0
#define ABS_Y 1
#define ABS_PRESSURE 24
#define ABS_MT_SLOT 47
#define ABS_MT_TRACKING_ID 57
#define BTN_TOOL_RUBBER 321
#define BTN_TOUCH 330
#define KEY_POWER 116
#define EVIOCGRAB 0x40044590
#define MAX_SLOTS 16
#define DIGI_MAX_X 11180
#define DIGI_MAX_Y 15340
#define TRAIL 11

struct input_event { struct timeval time; uint16_t type; uint16_t code; int32_t value; };
struct pt { int x, y, r, valid; };

static volatile sig_atomic_t g_quit = 0;
static void on_term(int sig) { (void)sig; g_quit = 1; }

static int W, H, STRIDE, BPP;
static unsigned char *FB;

static void put_px(int x, int y, int black) {
    if (x < 0 || y < 0 || x >= W || y >= H) return;
    unsigned char v = black ? 0x00 : 0xFF;
    unsigned char *p = FB + (size_t)y * STRIDE + (size_t)x * BPP;
    memset(p, v, BPP);
    if (BPP == 4) p[3] = 0xFF;
}

static void stamp(int cx, int cy, int r, int black) {
    for (int dy = -r; dy <= r; dy++)
        for (int dx = -r; dx <= r; dx++)
            if (dx * dx + dy * dy <= r * r)
                put_px(cx + dx, cy + dy, black);
}

static void line(int x0, int y0, int x1, int y1, int r, int black) {
    int dx = abs(x1 - x0), dy = abs(y1 - y0);
    int steps = dx > dy ? dx : dy;
    if (steps < 1) steps = 1;
    for (int i = 0; i <= steps; i++)
        stamp(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps, r, black);
}

static void ellipse(int cx, int cy, int rx, int ry, int black) {
    for (int y = -ry; y <= ry; y++)
        for (int x = -rx; x <= rx; x++)
            if (x*x*ry*ry + y*y*rx*rx <= rx*rx*ry*ry)
                put_px(cx + x, cy + y, black);
}

static void foot(int x, int y, int i) {
    int side = (i & 1) ? 1 : -1;
    int tilt = side * 5;
    ellipse(x, y, 8, 12, 1);
    ellipse(x + side * 8, y - 15, 5, 7, 1);
    ellipse(x + side * 2 + tilt, y - 25, 3, 4, 1);
    ellipse(x + side * 9 + tilt, y - 27, 3, 4, 1);
    ellipse(x + side * 15 + tilt, y - 23, 2, 3, 1);
}

static void dirty_add(int *x0, int *y0, int *x1, int *y1, int x, int y, int m) {
    if (x - m < *x0) *x0 = x - m;
    if (y - m < *y0) *y0 = y - m;
    if (x + m > *x1) *x1 = x + m;
    if (y + m > *y1) *y1 = y + m;
}

static int open_input(const char *needle) {
    char path[64], name[128], lower[128];
    for (int i = 0; i < 8; i++) {
        snprintf(path, sizeof path, "/sys/class/input/event%d/device/name", i);
        FILE *f = fopen(path, "r"); if (!f) continue;
        if (!fgets(name, sizeof name, f)) { fclose(f); continue; }
        fclose(f); memset(lower, 0, sizeof lower);
        for (size_t j = 0; j < sizeof lower - 1 && name[j]; j++)
            lower[j] = (name[j] >= 'A' && name[j] <= 'Z') ? name[j] + 32 : name[j];
        if (!strstr(lower, needle)) continue;
        snprintf(path, sizeof path, "/dev/input/event%d", i);
        int fd = open(path, O_RDONLY | O_NONBLOCK);
        if (fd >= 0) { int one = 1; ioctl(fd, EVIOCGRAB, &one); fprintf(stderr, "drawlab: %s -> %s\n", needle, path); }
        return fd;
    }
    return -1;
}

static void drain_nonpen(int pwr_fd, int touch_fd) {
    struct input_event evs[64];
    if (pwr_fd >= 0) { ssize_t n; while ((n = read(pwr_fd, evs, sizeof evs)) > 0)
        for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++)
            if (evs[i].type == EV_KEY && evs[i].code == KEY_POWER && evs[i].value == 1) g_quit = 1; }
    static int slot_active[MAX_SLOTS] = {0}; static int cur_slot = 0;
    if (touch_fd >= 0) { ssize_t n; while ((n = read(touch_fd, evs, sizeof evs)) > 0)
        for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++) {
            if (evs[i].type == EV_ABS && evs[i].code == ABS_MT_SLOT) cur_slot = evs[i].value < 0 ? 0 : (evs[i].value >= MAX_SLOTS ? MAX_SLOTS-1 : evs[i].value);
            else if (evs[i].type == EV_ABS && evs[i].code == ABS_MT_TRACKING_ID) {
                slot_active[cur_slot] = evs[i].value != -1;
                int fingers = 0; for (int s = 0; s < MAX_SLOTS; s++) fingers += slot_active[s];
                if (fingers >= 5) g_quit = 1;
            }
        }}
}

int main(void) {
    signal(SIGTERM, on_term); signal(SIGINT, on_term);
    if (quill_init() != 0) return 1;
    W = quill_width(); H = quill_height(); STRIDE = quill_stride(); BPP = STRIDE / (W ? W : 1); FB = quill_buffer();
    fprintf(stderr, "drawlab: %dx%d stride %d bpp %d fmt %d\n", W, H, STRIDE, BPP, quill_format());
    memset(FB, 0xFF, (size_t)STRIDE * H);
    quill_swap(0, 0, W, H, 3, 1);

    int pen_fd = open_input("marker"), pwr_fd = open_input("powerkey"), touch_fd = open_input("touch");
    if (pen_fd < 0) return 1;
    struct pollfd pfds[3] = {{.fd=pen_fd,.events=POLLIN},{.fd=pwr_fd,.events=POLLIN},{.fd=touch_fd,.events=POLLIN}};

    int rx=0, ry=0, pressure=0, touching=0, eraser=0, have=0, lx=-1, ly=-1, lr=3;
    int last_fx=-9999, last_fy=-9999, fi=0;
    struct pt q[TRAIL]; memset(q, 0, sizeof q); int qn = 0, qi = 0;
    int dx0=1<<30, dy0=1<<30, dx1=-1, dy1=-1;
    struct timeval last_flush = {0,0};

    while (!g_quit) {
        poll(pfds, 3, 4);
        drain_nonpen(pwr_fd, touch_fd);
        struct input_event evs[96];
        ssize_t n = read(pen_fd, evs, sizeof evs);
        for (int i = 0; i < (int)((n > 0 ? n : 0) / sizeof(struct input_event)); i++) {
            struct input_event *e = &evs[i];
            if (e->type == EV_ABS && e->code == ABS_X) { rx = e->value; have = 1; }
            else if (e->type == EV_ABS && e->code == ABS_Y) { ry = e->value; have = 1; }
            else if (e->type == EV_ABS && e->code == ABS_PRESSURE) { pressure = e->value; have = 1; }
            else if (e->type == EV_KEY && e->code == BTN_TOOL_RUBBER) eraser = e->value;
            else if (e->type == EV_KEY && e->code == BTN_TOUCH) { touching = e->value; have = 1; }
            else if (e->type == EV_SYN && e->code == SYN_REPORT && have) {
                have = 0;
                int x = (int)((int64_t)rx * (W - 1) / DIGI_MAX_X);
                int y = (int)((int64_t)ry * (H - 1) / DIGI_MAX_Y);
                if (touching && pressure > 40) {
                    int r = eraser ? 22 : 2 + pressure * 3 / 4096;
                    if (lx >= 0) line(lx, ly, x, y, r, !eraser); else stamp(x, y, r, !eraser);
                    dirty_add(&dx0,&dy0,&dx1,&dy1,x,y,r+36); if (lx >= 0) dirty_add(&dx0,&dy0,&dx1,&dy1,lx,ly,r+36);
                    if (!eraser) {
                        int fdx = x - last_fx, fdy = y - last_fy;
                        if (fdx*fdx + fdy*fdy > 120*120) { foot(x+52, y-38, fi++); last_fx=x; last_fy=y; dirty_add(&dx0,&dy0,&dx1,&dy1,x+52,y-38,42); }
                        struct pt cur = {x,y,r+13,1};
                        if (qn >= TRAIL) {
                            struct pt old = q[qi];
                            struct pt old2 = q[(qi + 1) % TRAIL];
                            if (old.valid) {
                                if (old2.valid) line(old.x, old.y, old2.x, old2.y, old.r, 0); else stamp(old.x, old.y, old.r, 0);
                                dirty_add(&dx0,&dy0,&dx1,&dy1,old.x,old.y,old.r+3);
                                if (old2.valid) dirty_add(&dx0,&dy0,&dx1,&dy1,old2.x,old2.y,old.r+3);
                            }
                        } else qn++;
                        q[qi] = cur; qi = (qi + 1) % TRAIL;
                    }
                    lx=x; ly=y; lr=r;
                } else {
                    lx=ly=-1; qn=qi=0; memset(q,0,sizeof q); last_fx=last_fy=-9999;
                }
            }
        }
        if (dx1 >= 0) {
            struct timeval now; gettimeofday(&now, NULL);
            long ms = (now.tv_sec-last_flush.tv_sec)*1000 + (now.tv_usec-last_flush.tv_usec)/1000;
            if (ms >= 8) {
                if (dx0 < 0) dx0=0; if (dy0 < 0) dy0=0; if (dx1 >= W) dx1=W-1; if (dy1 >= H) dy1=H-1;
                quill_swap(dx0, dy0, dx1-dx0+1, dy1-dy0+1, 0, 0);
                dx0=dy0=1<<30; dx1=dy1=-1; last_flush=now;
            }
        }
        quill_process_events();
    }
    return 0;
}
