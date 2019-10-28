import datetime
import time

from pathlib import Path

import pystray

from PIL import Image
from playsound import playsound

SCRIPT_PATH = Path(__file__).absolute().parent

ICON_FILE = SCRIPT_PATH / 'icon.png'

ONE_CHIME = str(SCRIPT_PATH / 'chime_once.wav')
TWO_CHIMES = str(SCRIPT_PATH / 'chime_twice.wav')

WATCH_TIMES = {
    'first': range(2000, 2400),
    'middle': range(0000, 400),
    'morning': range(400, 800),
    'forenoon': range(800, 1200),
    'afternoon': range(1200, 1600),
    'first dog': range(1600, 1800),
    'last dog': range(1800, 2000)
}

INT_TO_WORDS = {
    1: 'One bell',
    2: 'Two bells',
    3: 'Three bells',
    4: 'Four bells',
    5: 'Five bells',
    6: 'Six bells',
    7: 'Seven bells',
    8: 'Eight bells'
}

muted = False
shutting_down = False

icon = pystray.Icon(name='Watch Bells')
icon.icon = Image.open(ICON_FILE)


def create_menu():
    return pystray.Menu(
        pystray.MenuItem(
            text='Mute',
            action=toggle_mute,
            checked=lambda item: muted
        ),
        pystray.MenuItem(
            text='Quit',
            action=quit_program
        )
    )


def toggle_mute():
    global muted
    muted = not muted


def quit_program():
    global shutting_down
    shutting_down = True
    icon.stop()


icon.menu = create_menu()


def watch_clock(icon):
    icon.visible = True
    while not shutting_down:
        next_bell = next_half_hour()
        seconds_to_wait = (
            (next_bell - datetime.datetime.now())
            .total_seconds()
        )
        # Waiting 10 seconds at most means a maximum of 10 seconds
        # delay for the program to shut down
        if seconds_to_wait > 10:
            time.sleep(10)
        else:
            time.sleep(seconds_to_wait)
            watch = watch_for_datetime(next_bell)
            bells = bells_for_datetime(next_bell)
            icon.title = f'{INT_TO_WORDS[bells]} of the {watch} watch'
            if not muted:
                ring_bells(bells)


def next_half_hour():
    # Be deliberately timezone-naive and just read computer clock
    dt = datetime.datetime.now()
    dt = dt.replace(second=0, microsecond=0)
    if dt.minute < 30:
        dt = dt.replace(minute=30)
    else:
        dt = dt.replace(minute=0) + datetime.timedelta(hours=1)
    return dt


def watch_for_datetime(dt):
    timestamp = int(f'{dt.hour}{dt.minute:02d}')
    if timestamp not in range(0, 2400):
        raise ValueError(f'Invalid timestamp {timestamp}')
    for watch, timerange in WATCH_TIMES.items():
        if timestamp in timerange:
            return watch


def bells_for_datetime(dt):
    bells = dt.hour % 4 * 2
    if dt.minute == 30:
        bells += 1
    if bells == 0:
        bells = 8
    if watch_for_datetime(dt) == 'last dog' and bells != 8:
        bells -= 4
    return bells


def ring_bells(bells):
    pairs = bells // 2
    single = bells % 2

    for i in range(pairs):
        playsound(TWO_CHIMES)

    if single:
        playsound(ONE_CHIME)


if '__main__' == __name__:
    dt = datetime.datetime.now()
    watch = watch_for_datetime(dt)
    bells = bells_for_datetime(dt)
    icon.title = f'{INT_TO_WORDS[bells]} of the {watch} watch'
    icon.run(setup=watch_clock)
