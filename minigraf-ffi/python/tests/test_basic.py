import json
import os
import pytest
from minigraf import MiniGrafDb, MiniGrafError


def test_open_in_memory():
    db = MiniGrafDb.open_in_memory()
    assert db is not None


def test_transact_and_query():
    db = MiniGrafDb.open_in_memory()
    result = json.loads(db.execute('(transact [[:alice :name "Alice"]])'))
    assert "transacted" in result

    result = json.loads(db.execute("(query [:find ?n :where [?e :name ?n]])"))
    assert result["variables"] == ["?n"]
    assert result["results"][0][0] == "Alice"


def test_invalid_datalog_raises():
    db = MiniGrafDb.open_in_memory()
    with pytest.raises(Exception):
        db.execute("not valid datalog !!!")


def test_file_backed_roundtrip(tmp_path):
    path = str(tmp_path / "test.graph")

    db = MiniGrafDb.open(path)
    db.execute('(transact [[:bob :name "Bob"]])')
    db.checkpoint()
    del db

    db2 = MiniGrafDb.open(path)
    result = json.loads(db2.execute("(query [:find ?n :where [?e :name ?n]])"))
    assert result["results"][0][0] == "Bob"
