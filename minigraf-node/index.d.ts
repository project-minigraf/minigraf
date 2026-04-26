/* tslint:disable */
/* eslint-disable */

export class MiniGrafDb {
  constructor(path: string)
  static inMemory(): MiniGrafDb
  execute(datalog: string): string
  checkpoint(): void
}
