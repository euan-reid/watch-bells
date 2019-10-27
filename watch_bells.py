import pystray
from PIL import Image, ImageDraw

icon = pystray.Icon('Watch Bells')

def quit_program():
    exit(0)

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
    icon.run()
