"""The server and the agent run on a Python with nothing installed (the one a computer has, or the one
Switchboard brings). Every import in them is Python's own library or a file beside them.

0.9.0 turned iPhone notifications off on a server that had them: the Python it brought had no `cryptography`
package, and the one test that would have said so was skipped wherever the package was missing."""
import ast, sys, unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# Used only when present, inside a try. Never needed.
OPTIONAL = {'zeroconf'}  # server/bonjour.py: announced with it when it is there, with the system's own tool when not


def imports(path):
    for node in ast.walk(ast.parse(path.read_text(encoding='utf-8'), str(path))):
        if isinstance(node, ast.Import):
            for a in node.names:
                yield a.name.split('.')[0], node.lineno
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            yield node.module.split('.')[0], node.lineno


class PlainPython(unittest.TestCase):
    def check(self, folder):
        std = getattr(sys, 'stdlib_module_names', None)
        if std is None:
            self.skipTest('this Python cannot list its own library (3.10 and later can)')
        files = sorted((ROOT / folder).glob('*.py'))
        self.assertTrue(files, folder)
        own = {p.stem for p in files}
        bad = [f'{p.relative_to(ROOT)}:{line}: {name}' for p in files for name, line in imports(p)
               if name not in std and name not in own and name not in OPTIONAL]
        self.assertEqual(bad, [], 'these need a package that is not part of Python')

    def test_the_server(self):
        self.check('server')

    def test_the_agent(self):
        self.check('agent')


if __name__ == '__main__':
    unittest.main()
