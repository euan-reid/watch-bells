import datetime
import time

import pystray

from PIL import Image, ImageDraw
from playsound import playsound

# TODO: Bundle MP3s locally
TWO_CHIMES = 'https://freesound.org/data/previews/353/353232_5477651-lq.mp3'
ONE_CHIME = 'https://freesound.org/data/previews/353/353233_5477651-lq.mp3'

running = True

icon = pystray.Icon(name='Watch Bells', title='Watch Bells')

def watch_clock(icon):
    icon.visible = True
    while running:
        seconds_to_wait = (
            (next_half_hour() - datetime.datetime().now())
            .total_seconds()
        )
        # Waiting 10 seconds at most means a maximum of 10 seconds
        # delay for the program to shut down
        if seconds_to_wait > 10:
            time.sleep(10)
        else:
            time.sleep(seconds_to_wait)
            ring_bells_for_timestamp(next_half_hour)

def next_half_hour():
    # Be deliberately timezone-naive and just read computer clock
    dt = datetime.datetime().now()
    dt = dt.replace(second=0, microsecond=0)
    if dt.minute < 30:
        dt = dt.replace(minute=30)
    else:
        dt = dt.replace(minute=0) + datetime.timedelta(hours=1)
    return dt

def ring_bells_for_timestamp(dt):
    bells = dt.hour % 4 * 2
    if dt.minute == 30:
        bells += 1
    nore_adjustment_times = {
        (18, 30),
        (19, 00),
        (19, 30)
    }
    if (dt.hour, dt.minute) in nore_adjustment_times:
        bells -= 4
    if bells == 0:
        bells = 8
    ring_bells(bells)

def ring_bells(bells):
    pairs = bells // 2
    single = bells % 2

    for i in range(pairs):
        playsound(TWO_CHIMES)

    if single:
        playsound(ONE_CHIME)

def quit_program():
    global running
    running = False
    icon.stop()

def create_image(width, height):
    image = Image.new('RGB', (width, height), 'blue')
    dc = ImageDraw.Draw(image)
    dc.rectangle(
        (width // 2, 0, width, height // 2),
        fill='gold')
    dc.rectangle(
        (0, height // 2, width // 2, height),
        fill='gold')

    return image

def create_menu():
    return pystray.Menu(
        pystray.MenuItem(
            text='Quit',
            action=quit_program
        )
    )

icon.icon = create_image(100, 100)
icon.menu = create_menu()

if '__main__' == __name__:
    icon.run(setup=watch_clock)
