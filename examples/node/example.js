'use strict'

const { MiniGrafDb } = require('minigraf')

const db = MiniGrafDb.inMemory()

db.execute('(transact [[:alice :name "Alice"] [:alice :age 30]])')

const result = JSON.parse(db.execute('(query [:find ?e ?name :where [?e :name ?name]])'))
console.log(result)

db.checkpoint()
