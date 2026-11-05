import unittest

from run import select_cases


class CaseSelectionTests(unittest.TestCase):
    def test_subset_keeps_authoritative_matrix_order(self):
        cases=[{'id':'a'},{'id':'b'},{'id':'c'}]
        self.assertEqual(select_cases(cases,['c','a']),[cases[0],cases[2]])
        self.assertEqual(select_cases(cases,None),cases)

    def test_missing_empty_duplicate_and_malformed_requests_are_rejected(self):
        cases=[{'id':'a'},{'id':'b'}]
        for requested in [[],['unknown'],['a','a'],'a',{'a':True}]:
            with self.subTest(requested=requested),self.assertRaises(ValueError):
                select_cases(cases,requested)
