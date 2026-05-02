import uniffi.minigraf_ffi.MiniGrafDb;

public class Example {
    public static void main(String[] args) {
        MiniGrafDb db = MiniGrafDb.openInMemory();

        db.execute("(transact [[:alice :name \"Alice\"] [:alice :age 30]])");

        String result = db.execute("(query [:find ?e ?name :where [?e :name ?name]])");
        System.out.println(result);

        db.checkpoint();
        db.destroy();
    }
}
