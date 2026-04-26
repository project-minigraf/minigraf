from minigraf import MiniGrafDb

db = MiniGrafDb.open_in_memory()

db.execute('(transact [[:alice :name "Alice"] [:alice :age 30]])')

result = db.execute("(query [:find ?e ?name :where [?e :name ?name]])")
import json
print(json.loads(result))

db.checkpoint()
