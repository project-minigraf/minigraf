import unittest
from unittest.mock import patch

from run import memory_caps


class MemoryCapTests(unittest.TestCase):
    def test_pair_shares_host_budget_and_respects_each_bound_node(self):
        def node(path):
            return 'Node MemFree: 1000 kB' if 'node1' in str(path) else 'Node MemFree: 9000 kB'
        with patch('run.memory_available',return_value=8000*1024), patch('run.Path.read_text',node):
            bounds=memory_caps(['1','3'])
        self.assertEqual(bounds['child_caps_bytes'],[800*1024,3200*1024])

    def test_two_children_on_one_node_share_its_budget(self):
        with patch('run.memory_available',return_value=10000*1024), patch('run.Path.read_text',return_value='Node MemFree: 2000 kB'):
            bounds=memory_caps(['1','1'])
        self.assertEqual(bounds['child_caps_bytes'],[800*1024,800*1024])
