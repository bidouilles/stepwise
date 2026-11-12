pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "Dictionary of lists",
        "def create_dict(lines):\n  letters = {}   \n  for i in range(len(lines)):\n    add_line ( lines[i], i, letters )\n  return letters\n\ndef add_line(line,number,letters):\n  for car in line:          \n    letters.setdefault(car,[]).append(number)\n\nlines = [ \"aabb\" ]\nletters = create_dict ( lines )\nletters2 = letters\nadd_line(\"ba\",1,letters)\nprint(letters)\nprint(letters2)\n",
    ),
    (
        "Variables and aliasing",
        "a = [1, 2, 3]\nb = a          # b points to the SAME list\nc = a[:]       # c is a copy\n\nb.append(4)\nc.append(99)\n\nprint(a)\nprint(b)\nprint(c)\n",
    ),
    (
        "Loop and accumulator",
        "numbers = [4, 8, 15, 16]\ntotal = 0\n\nfor n in numbers:\n    total = total + n\n\naverage = total / len(numbers)\nprint(\"average:\", average)\n",
    ),
    (
        "Function call",
        "def area(width, height):\n    result = width * height\n    return result\n\nw = 3\nh = 5\na = area(w, h)\nprint(a)\n",
    ),
    (
        "Recursion: factorial",
        "def factorial(n):\n    if n <= 1:\n        return 1\n    return n * factorial(n - 1)\n\nanswer = factorial(4)\nprint(answer)\n",
    ),
    (
        "Mutating a list in a function",
        "def double_all(values):\n    for i in range(len(values)):\n        values[i] = values[i] * 2\n\nscores = [1, 2, 3]\ndouble_all(scores)\nprint(scores)\n",
    ),
    (
        "Nested lists",
        "row = [0, 0]\ngrid = [row, row]      # the same row, twice!\ngrid[0][0] = 7\nprint(grid)\n\ngood = [[0, 0] for _ in range(2)]\ngood[0][0] = 7\nprint(good)\n",
    ),
    (
        "Dictionary",
        "ages = {\"Ana\": 12, \"Ben\": 14}\nages[\"Chloe\"] = 13\n\nfor name in ages:\n    print(name, \"is\", ages[name])\n\noldest = max(ages, key=ages.get)\nprint(\"oldest:\", oldest)\n",
    ),
    (
        "Classes and objects",
        "class Dog:\n    def __init__(self, name):\n        self.name = name\n        self.tricks = []\n\n    def learn(self, trick):\n        self.tricks.append(trick)\n\nrex = Dog(\"Rex\")\nmia = Dog(\"Mia\")\nrex.learn(\"sit\")\nrex.learn(\"roll\")\nprint(rex.name, rex.tricks)\nprint(mia.name, mia.tricks)\n",
    ),
    (
        "Linked list",
        "class Node:\n    def __init__(self, value, next=None):\n        self.value = value\n        self.next = next\n\nhead = None\nfor v in [3, 2, 1]:\n    head = Node(v, head)\n\nnode = head\nwhile node is not None:\n    print(node.value)\n    node = node.next\n",
    ),
    (
        "Input and an error",
        "name = input(\"Your name? \")\nage = int(input(\"Your age? \"))\nprint(\"Hello\", name)\nprint(\"Next year you will be\", age + 1)\n\nitems = [1, 2, 3]\nprint(items[5])     # oops\n",
    ),
];
