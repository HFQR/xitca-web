use std::{
    future::IntoFuture,
    io::Write,
    process::{Command, Stdio},
};

use criterion::{Criterion, criterion_group, criterion_main};
use tokio::runtime::Runtime;
use xitca_postgres::{Execute, Postgres, Statement, iter::AsyncLendingIterator};

const PG_USER: &str = "bench_user";
const PG_DB: &str = "bench_db";
const PG_CONN: &str = "postgres://bench_user@localhost:5432/bench_db";

const FORTUNE_SQL: &str = "\
CREATE TABLE IF NOT EXISTS fortune (\
  id integer NOT NULL,\
  message varchar(2048) NOT NULL,\
  PRIMARY KEY (id)\
);\
DELETE FROM fortune;\
INSERT INTO fortune (id, message) VALUES (1, 'fortune: No such file or directory');\
INSERT INTO fortune (id, message) VALUES (2, 'A computer scientist is someone who fixes things that aren''t broken.');\
INSERT INTO fortune (id, message) VALUES (3, 'After enough decimal places, nobody gives a damn.');\
INSERT INTO fortune (id, message) VALUES (4, 'A bad random number generator: 1, 1, 1, 1, 1, 4.33e+67, 1, 1, 1');\
INSERT INTO fortune (id, message) VALUES (5, 'A computer program does what you tell it to do, not what you want it to do.');\
INSERT INTO fortune (id, message) VALUES (6, 'Emacs is a nice operating system, but I prefer UNIX. — Tom Christaensen');\
INSERT INTO fortune (id, message) VALUES (7, 'Any program that runs right is obsolete.');\
INSERT INTO fortune (id, message) VALUES (8, 'A list is only as strong as its weakest link. — Donald Knuth');\
INSERT INTO fortune (id, message) VALUES (9, 'Feature: A bug with seniority.');\
INSERT INTO fortune (id, message) VALUES (10, 'Computers make very fast, very accurate mistakes.');\
INSERT INTO fortune (id, message) VALUES (11, '<script>alert(\"This should not be displayed in a browser alert box.\");</script>');\
INSERT INTO fortune (id, message) VALUES (12, 'フレームワークのベンチマーク');\
";

struct PgInstance {
    data_dir: tempfile::TempDir,
}

impl PgInstance {
    fn start() -> Self {
        let data_dir = tempfile::tempdir().expect("failed to create tempdir");
        let dir = data_dir.path();

        // initdb
        let out = Command::new("initdb")
            .args(["-D", dir.to_str().unwrap(), "--no-locale", "-E", "UTF8"])
            .output()
            .expect("initdb not found");
        assert!(
            out.status.success(),
            "initdb failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // start postgres
        let child = Command::new("pg_ctl")
            .args([
                "start",
                "-D",
                dir.to_str().unwrap(),
                "-l",
                dir.join("logfile").to_str().unwrap(),
                "-o",
                "-k /tmp -h localhost",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("pg_ctl not found");
        let status = child.wait_with_output().expect("pg_ctl wait failed");
        assert!(status.status.success(), "pg_ctl start failed");

        // wait for ready
        for _ in 0..30 {
            let out = Command::new("pg_isready").args(["-h", "localhost"]).output().unwrap();
            if out.status.success() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        // create user and database
        run_cmd("createuser", &["-h", "localhost", PG_USER]);
        run_cmd("createdb", &["-h", "localhost", "-O", PG_USER, PG_DB]);

        // populate tables
        let mut psql = Command::new("psql")
            .args(["-h", "localhost", "-d", PG_DB, "-U", PG_USER])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("psql not found");
        psql.stdin.as_mut().unwrap().write_all(FORTUNE_SQL.as_bytes()).unwrap();
        let out = psql.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "psql populate failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        PgInstance { data_dir }
    }
}

impl Drop for PgInstance {
    fn drop(&mut self) {
        let _ = Command::new("pg_ctl")
            .args(["stop", "-D", self.data_dir.path().to_str().unwrap(), "-m", "immediate"])
            .output();
    }
}

fn run_cmd(cmd: &str, args: &[&str]) {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|_| panic!("{cmd} not found"));
    assert!(
        out.status.success(),
        "{cmd} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn bench_fortune(c: &mut Criterion) {
    let _pg = PgInstance::start();
    let rt = Runtime::new().unwrap();

    let (cli, drv) = rt.block_on(Postgres::new(PG_CONN).connect()).expect("connect failed");
    let handle = rt.spawn(drv.into_future());

    // prepare and leak to get an unguarded Statement (no borrow on cli)
    let stmt = rt
        .block_on(Statement::named("SELECT id, message FROM fortune", &[]).execute(&cli))
        .expect("prepare failed")
        .leak();

    c.bench_function("fortune_select_all", |b| {
        b.to_async(&rt).iter(|| async {
            let mut stream = stmt.query(&cli).await.unwrap();
            let mut count = 0u32;
            while let Some(row) = stream.try_next().await.unwrap() {
                let _id: i32 = row.get(0);
                let _msg: &str = row.get(1);
                count += 1;
            }
            assert_eq!(count, 12);
        });
    });

    drop(cli);
    rt.block_on(handle).unwrap().unwrap();
}

criterion_group!(benches, bench_fortune);
criterion_main!(benches);
