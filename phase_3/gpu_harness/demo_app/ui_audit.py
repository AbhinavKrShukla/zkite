"""Device regression fixture: controls, shrinking lists, modal input and visuals.

Use the native shell or copy to main.py in an isolated zkite project. Interaction
results are printed as UIAUDIT lines for adb logcat assertions.
"""
from zkite import (
    Anim, Button, Checkbox, Column, Grid, Icon, Image, List, Overlay, Radio,
    Row, ScrollView, Select, Slider, Stack, Switch, SystemBars, Text, TextField, store,
)


@store
class Audit:
    page: str = 'Controls'
    enabled: bool = True
    checked: bool = False
    radio: bool = False
    value: float = 25
    count: int = 1000
    taps: int = 0
    text: str = ''
    choice: str = 'One'
    modal: bool = False


def update(s, **values):
    print('UIAUDIT', values)
    s.update(**values)


def button(label, action, **props):
    return Button('bg-indigo-600 text-white rounded-xl px-3 py-3', label, on_tap=action, **props)


def view(s: Audit):
    def navigate(page):
        return lambda: update(s, page=page)

    body = []
    if s.page == 'Controls':
        body = [
            Text('text-lg font-bold', f'Taps: {s.taps}'),
            button('Enable' if not s.enabled else 'Disable', lambda: update(s, enabled=not s.enabled)),
            button('Count', lambda: update(s, taps=s.taps+1), disabled=not s.enabled),
            Switch('', Text('', 'Switch'), checked=s.checked, disabled=not s.enabled,
                   on_change=lambda v: update(s, checked=v)),
            Checkbox('', Text('', 'Checkbox'), checked=s.checked, disabled=not s.enabled,
                     on_change=lambda v: update(s, checked=v)),
            Radio('', Text('', 'Radio'), checked=s.radio, disabled=not s.enabled,
                  on_change=lambda v: update(s, radio=v)),
            Text('', f'Value: {s.value:.0f}'),
            Slider('h-12', value=s.value, disabled=not s.enabled, on_change=lambda v: update(s, value=v)),
            Select('bg-white text-slate-900 p-3 rounded-xl', options=['One', 'Two', 'Three'],
                   value=s.choice, disabled=not s.enabled, on_change=lambda v: update(s, choice=v)),
        ]
    elif s.page == 'Lists':
        body = [
            Row('gap-2 shrink-0', button('Short list', lambda: update(s, count=3)),
                button('1000 rows', lambda: update(s, count=1000))),
            List('flex-1 bg-white rounded-xl', key='audit-list', count=s.count, item_height=56,
                 builder=lambda i: Row('h-[56px] shrink-0 items-center px-4', Text('', f'Item {i}'))),
        ]
    elif s.page == 'Input':
        body = [
            Text('text-lg', f'Text: {s.text}'),
            TextField('bg-white text-slate-900 p-3 rounded-xl h-12', key='entry', value=s.text,
                      placeholder='Type here', on_change=lambda v: update(s, text=v)),
            TextField('bg-white p-3 rounded-xl h-12', placeholder='Disabled input', disabled=True),
            button('Open modal', lambda: update(s, modal=True)),
            button('Count', lambda: update(s, taps=s.taps+1)),
        ]
    else:
        body = [
            Grid('grid-cols-2 gap-3', *[Column('bg-white rounded-xl p-4', Icon('text-indigo-600', name='check'),
                 Text('font-bold', f'Card {i}')) for i in range(4)]),
            ScrollView('h-20 shrink-0 gap-3', *[Text('shrink-0 bg-indigo-100 p-4 rounded-xl', f'Tile {i}') for i in range(20)], horizontal=True),
            Stack('h-24', Text('bg-indigo-100 p-3', 'Stack'), Text('text-indigo-800 p-8', 'Layer')),
            Image('w-20 h-20', src='cyan16.png'),
            Text('p-4 bg-indigo-600 text-white rounded-xl', 'Animated entrance', key='entrance',
                 anim=Anim.enter(opacity=(0, 1), translate_y=(12, 0), ms=250)),
        ]
    return SystemBars(Column('flex-1 min-h-0 bg-slate-100 text-slate-900 p-4 gap-4',
        Text('text-2xl font-bold shrink-0', 'ZKite UI audit'),
        Row('gap-2 shrink-0', *[button(page, navigate(page)) for page in ('Controls', 'Lists', 'Input', 'Visuals')]),
        *body,
        Overlay('bg-white rounded-2xl p-6 flex-col gap-4 w-[320px]',
                Text('text-xl font-bold', 'Modal'),
                Select('p-3 bg-slate-100', options=['One', 'Two', 'Three'], value=s.choice,
                       on_change=lambda v: update(s, choice=v)),
                button('Close modal', lambda: update(s, modal=False)),
                on_dismiss=lambda: update(s, modal=False)) if s.modal else None,
    ), status_color='#f1f5f9', navigation_color='#f1f5f9')
