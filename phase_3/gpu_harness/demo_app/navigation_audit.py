"""Native navigation/input regression fixture; copy into a test project's main.py."""
from zkite import Button, Column, List, Nav, Navigator, SystemBars, Text, TextField, store

@store
class State:
    text: str = 'Café 😀 hello'

def action(label, callback):
    return Button('p-4 bg-indigo-600 text-white rounded-xl shrink-0', label, on_tap=callback)

def home(nav: Navigator):
    return Column('flex-1 min-h-0 gap-4', Text('text-xl', 'Navigation home'),
        action('Open detail', lambda: nav.push(detail, number=1)),
        List('flex-1 bg-white', key='rows', count=100, item_height=48,
             builder=lambda i: Text('h-12 p-3 shrink-0', f'Home row {i}')))

def detail(nav: Navigator, state: State, number: int):
    def change(text):
        print('NAVINPUT', repr(text))
        state.update(text=text)
    return Column('flex-1 gap-4', Text('text-xl', f'Detail {number}'),
        action('Push detail', lambda: nav.push(detail, number=number+1)),
        action('Back', nav.pop), action('Root', nav.pop_to_root),
        Text('flex-1', state.text),
        TextField('h-16 bg-white text-slate-900 px-6 py-4 rounded-xl',
                  key='editor', value=state.text, on_change=change))

def view():
    return SystemBars(Column('flex-1 min-h-0 p-4 bg-slate-100 text-slate-900',
        Nav('flex-1', home, detail)), status_color='#f1f5f9', navigation_color='#f1f5f9')
