use xitca_router::{InsertError, Router};

struct InsertTest(Vec<(&'static str, Result<(), InsertError>)>);

impl InsertTest {
    fn run(self) {
        let mut router = Router::new();
        for (route, expected) in self.0 {
            let got = router.insert(route, route.to_owned());
            assert_eq!(got, expected, "{route}");
        }
    }
}

fn conflict(with: &'static str) -> InsertError {
    InsertError::Conflict { with: with.into() }
}

// Regression test for https://github.com/ibraheemdev/matchit/issues/84.
#[test]
fn missing_leading_slash_suffix() {
    InsertTest(vec![("/{foo}", Ok(())), ("/{foo}suffix", Ok(()))]).run();
    InsertTest(vec![("{foo}", Ok(())), ("{foo}suffix", Ok(()))]).run();
}

// Regression test for https://github.com/ibraheemdev/matchit/issues/82.
#[test]
fn missing_leading_slash_conflict() {
    InsertTest(vec![("{foo}/", Ok(())), ("foo/", Ok(()))]).run();
    InsertTest(vec![("foo/", Ok(())), ("{foo}/", Ok(()))]).run();
}

#[test]
fn wildcard_conflict() {
    InsertTest(vec![
        ("/cmd/{tool}/{sub}", Ok(())),
        ("/cmd/vet", Ok(())),
        ("/foo/bar", Ok(())),
        ("/foo/{name}", Ok(())),
        ("/foo/{names}", Err(conflict("/foo/{name}"))),
        ("/cmd/{*path}", Err(conflict("/cmd/{tool}/{sub}"))),
        ("/cmd/{xxx}/names", Ok(())),
        ("/cmd/{tool}/{xxx}/foo", Ok(())),
        ("/src/{*filepath}", Ok(())),
        ("/src/{file}", Err(conflict("/src/{*filepath}"))),
        ("/src/static.json", Ok(())),
        ("/src/$filepathx", Ok(())),
        ("/src/", Ok(())),
        ("/src/foo/bar", Ok(())),
        ("/src1/", Ok(())),
        ("/src1/{*filepath}", Ok(())),
        ("/src2{*filepath}", Ok(())),
        ("/src2/{*filepath}", Ok(())),
        ("/src2/", Ok(())),
        ("/src2", Ok(())),
        ("/src3", Ok(())),
        ("/src3/{*filepath}", Ok(())),
        ("/search/{query}", Ok(())),
        ("/search/valid", Ok(())),
        ("/user_{name}", Ok(())),
        ("/user_x", Ok(())),
        ("/user_{bar}", Err(conflict("/user_{name}"))),
        ("/id{id}", Ok(())),
        ("/id/{id}", Ok(())),
        ("/x/{id}", Ok(())),
        ("/x/{id}/", Ok(())),
        ("/x/{id}y", Ok(())),
        ("/x/{id}y/", Ok(())),
        ("/x/{id}y", Err(conflict("/x/{id}y"))),
        ("/x/x{id}", Err(conflict("/x/{id}y/"))),
        ("/x/x{id}y", Err(conflict("/x/{id}y/"))),
        ("/y/{id}", Ok(())),
        ("/y/{id}/", Ok(())),
        ("/y/y{id}", Ok(())),
        ("/y/y{id}/", Ok(())),
        ("/y/{id}y", Err(conflict("/y/y{id}/"))),
        ("/y/{id}y/", Err(conflict("/y/y{id}/"))),
        ("/y/x{id}y", Err(conflict("/y/y{id}/"))),
        ("/z/x{id}y", Ok(())),
        ("/z/{id}", Ok(())),
        ("/z/{id}y", Err(conflict("/z/x{id}y"))),
        ("/z/x{id}", Err(conflict("/z/x{id}y"))),
        ("/z/y{id}", Err(conflict("/z/x{id}y"))),
        ("/z/x{id}z", Err(conflict("/z/x{id}y"))),
        ("/z/z{id}y", Err(conflict("/z/x{id}y"))),
        ("/bar/{id}", Ok(())),
        ("/bar/x{id}y", Ok(())),
    ])
    .run()
}

#[test]
fn prefix_suffix_conflict() {
    InsertTest(vec![
        ("/x1/{a}suffix", Ok(())),
        ("/x1/prefix{a}", Err(conflict("/x1/{a}suffix"))),
        ("/x1/prefix{a}suffix", Err(conflict("/x1/{a}suffix"))),
        ("/x1/suffix{a}prefix", Err(conflict("/x1/{a}suffix"))),
        ("/x1", Ok(())),
        ("/x1/", Ok(())),
        ("/x1/{a}", Ok(())),
        ("/x1/{a}/", Ok(())),
        ("/x1/{a}suffix/", Ok(())),
        ("/x2/{a}suffix", Ok(())),
        ("/x2/{a}", Ok(())),
        ("/x2/prefix{a}", Err(conflict("/x2/{a}suffix"))),
        ("/x2/prefix{a}suff", Err(conflict("/x2/{a}suffix"))),
        ("/x2/prefix{a}suffix", Err(conflict("/x2/{a}suffix"))),
        ("/x2/prefix{a}suffixy", Err(conflict("/x2/{a}suffix"))),
        ("/x2", Ok(())),
        ("/x2/", Ok(())),
        ("/x2/{a}suffix/", Ok(())),
        ("/x3/prefix{a}", Ok(())),
        ("/x3/{a}suffix", Err(conflict("/x3/prefix{a}"))),
        ("/x3/prefix{a}suffix", Err(conflict("/x3/prefix{a}"))),
        ("/x3/prefix{a}/", Ok(())),
        ("/x3/{a}", Ok(())),
        ("/x3/{a}/", Ok(())),
        ("/x4/prefix{a}", Ok(())),
        ("/x4/{a}", Ok(())),
        ("/x4/{a}suffix", Err(conflict("/x4/prefix{a}"))),
        ("/x4/suffix{a}p", Err(conflict("/x4/prefix{a}"))),
        ("/x4/suffix{a}prefix", Err(conflict("/x4/prefix{a}"))),
        ("/x4/prefix{a}/", Ok(())),
        ("/x4/{a}/", Ok(())),
        ("/x5/prefix1{a}", Ok(())),
        ("/x5/prefix2{a}", Ok(())),
        ("/x5/{a}suffix", Err(conflict("/x5/prefix1{a}"))),
        ("/x5/prefix{a}suffix", Err(conflict("/x5/prefix1{a}"))),
        ("/x5/prefix1{a}suffix", Err(conflict("/x5/prefix1{a}"))),
        ("/x5/prefix2{a}suffix", Err(conflict("/x5/prefix2{a}"))),
        ("/x5/prefix3{a}suffix", Err(conflict("/x5/prefix1{a}"))),
        ("/x5/prefix1{a}/", Ok(())),
        ("/x5/prefix2{a}/", Ok(())),
        ("/x5/prefix3{a}/", Ok(())),
        ("/x5/{a}", Ok(())),
        ("/x5/{a}/", Ok(())),
        ("/x6/prefix1{a}", Ok(())),
        ("/x6/prefix2{a}", Ok(())),
        ("/x6/{a}", Ok(())),
        ("/x6/{a}suffix", Err(conflict("/x6/prefix1{a}"))),
        ("/x6/prefix{a}suffix", Err(conflict("/x6/prefix1{a}"))),
        ("/x6/prefix1{a}suffix", Err(conflict("/x6/prefix1{a}"))),
        ("/x6/prefix2{a}suffix", Err(conflict("/x6/prefix2{a}"))),
        ("/x6/prefix3{a}suffix", Err(conflict("/x6/prefix1{a}"))),
        ("/x6/prefix1{a}/", Ok(())),
        ("/x6/prefix2{a}/", Ok(())),
        ("/x6/prefix3{a}/", Ok(())),
        ("/x6/{a}/", Ok(())),
        ("/x7/prefix{a}suffix", Ok(())),
        ("/x7/{a}suff", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/{a}suffix", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/{a}suffixy", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/{a}prefix", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/suffix{a}prefix", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/another{a}", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/suffix{a}", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}/", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}suff", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}suffix", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}suffixy", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix1{a}", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}/", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/{a}suffix/", Err(conflict("/x7/prefix{a}suffix"))),
        ("/x7/prefix{a}suffix/", Ok(())),
        ("/x7/{a}", Ok(())),
        ("/x7/{a}/", Ok(())),
        ("/x8/prefix{a}suffix", Ok(())),
        ("/x8/{a}", Ok(())),
        ("/x8/{a}suff", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/{a}suffix", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/{a}suffixy", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}/", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}suff", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}suffix", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}suffixy", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix1{a}", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}/", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/{a}suffix/", Err(conflict("/x8/prefix{a}suffix"))),
        ("/x8/prefix{a}suffix/", Ok(())),
        ("/x8/{a}/", Ok(())),
        ("/x9/prefix{a}", Ok(())),
        ("/x9/{a}suffix", Err(conflict("/x9/prefix{a}"))),
        ("/x9/prefix{a}suffix", Err(conflict("/x9/prefix{a}"))),
        ("/x9/prefixabc{a}suffix", Err(conflict("/x9/prefix{a}"))),
        ("/x9/pre{a}suffix", Err(conflict("/x9/prefix{a}"))),
        ("/x10/{a}", Ok(())),
        ("/x10/prefix{a}", Ok(())),
        ("/x10/{a}suffix", Err(conflict("/x10/prefix{a}"))),
        ("/x10/prefix{a}suffix", Err(conflict("/x10/prefix{a}"))),
        ("/x10/prefixabc{a}suffix", Err(conflict("/x10/prefix{a}"))),
        ("/x10/pre{a}suffix", Err(conflict("/x10/prefix{a}"))),
        ("/x11/{a}", Ok(())),
        ("/x11/{a}suffix", Ok(())),
        ("/x11/prx11fix{a}", Err(conflict("/x11/{a}suffix"))),
        ("/x11/prx11fix{a}suff", Err(conflict("/x11/{a}suffix"))),
        ("/x11/prx11fix{a}suffix", Err(conflict("/x11/{a}suffix"))),
        ("/x11/prx11fix{a}suffixabc", Err(conflict("/x11/{a}suffix"))),
        ("/x12/prefix{a}suffix", Ok(())),
        ("/x12/pre{a}", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefix{a}", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefixabc{a}", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/pre{a}suffix", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefix{a}suffix", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefixabc{a}suffix", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefix{a}suff", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefix{a}suffix", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/prefix{a}suffixabc", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/{a}suff", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/{a}suffix", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x12/{a}suffixabc", Err(conflict("/x12/prefix{a}suffix"))),
        ("/x13/{a}", Ok(())),
        ("/x13/prefix{a}suffix", Ok(())),
        ("/x13/pre{a}", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefix{a}", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefixabc{a}", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/pre{a}suffix", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefix{a}suffix", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefixabc{a}suffix", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefix{a}suff", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefix{a}suffix", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/prefix{a}suffixabc", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/{a}suff", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/{a}suffix", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x13/{a}suffixabc", Err(conflict("/x13/prefix{a}suffix"))),
        ("/x15/{*rest}", Ok(())),
        ("/x15/{a}suffix", Err(conflict("/x15/{*rest}"))),
        ("/x15/{a}suffix", Err(conflict("/x15/{*rest}"))),
        ("/x15/prefix{a}", Ok(())),
        ("/x16/{*rest}", Ok(())),
        ("/x16/prefix{a}suffix", Ok(())),
        ("/x17/prefix{a}/z", Ok(())),
        ("/x18/prefix{a}/z", Ok(())),
    ])
    .run()
}

#[test]
fn invalid_catchall() {
    InsertTest(vec![
        ("/non-leading-{*catchall}", Ok(())),
        ("/foo/bar{*catchall}", Ok(())),
        ("/src/{*filepath}x", Err(InsertError::InvalidCatchAll)),
        ("/src/{*filepath}/x", Err(InsertError::InvalidCatchAll)),
        ("/src2/", Ok(())),
        ("/src2/{*filepath}/x", Err(InsertError::InvalidCatchAll)),
        ("/relax/{*}", Ok(())),
        ("/relax2/{*}/x", Err(InsertError::InvalidCatchAll)),
    ])
    .run()
}

#[test]
fn catchall_root_conflict() {
    InsertTest(vec![("/", Ok(())), ("/{*filepath}", Ok(()))]).run()
}

#[test]
fn child_conflict() {
    InsertTest(vec![
        ("/cmd/vet", Ok(())),
        ("/cmd/{tool}", Ok(())),
        ("/cmd/{tool}/{sub}", Ok(())),
        ("/cmd/{tool}/misc", Ok(())),
        ("/cmd/{tool}/{bad}", Err(conflict("/cmd/{tool}/{sub}"))),
        ("/src/AUTHORS", Ok(())),
        ("/src/{*filepath}", Ok(())),
        ("/user_x", Ok(())),
        ("/user_{name}", Ok(())),
        ("/id/{id}", Ok(())),
        ("/id{id}", Ok(())),
        ("/{id}", Ok(())),
        ("/{*filepath}", Err(conflict("/{id}"))),
    ])
    .run()
}

#[test]
fn duplicates() {
    InsertTest(vec![
        ("/", Ok(())),
        ("/", Err(conflict("/"))),
        ("/doc/", Ok(())),
        ("/doc/", Err(conflict("/doc/"))),
        ("/src/{*filepath}", Ok(())),
        ("/src/{*filepath}", Err(conflict("/src/{*filepath}"))),
        ("/search/{query}", Ok(())),
        ("/search/{query}", Err(conflict("/search/{query}"))),
        ("/user_{name}", Ok(())),
        ("/user_{name}", Err(conflict("/user_{name}"))),
    ])
    .run()
}

#[test]
fn unnamed_param() {
    InsertTest(vec![
        ("/{}", Err(InsertError::InvalidParam)),
        ("/user{}/", Err(InsertError::InvalidParam)),
        ("/cmd/{}/", Err(InsertError::InvalidParam)),
        // ("/src/{*}", Err(InsertError::InvalidParam)),
    ])
    .run()
}

#[test]
fn double_params() {
    InsertTest(vec![
        ("/{foo}{bar}", Err(InsertError::InvalidParamSegment)),
        ("/{foo}{bar}/", Err(InsertError::InvalidParamSegment)),
        ("/{foo}{{*bar}/", Err(InsertError::InvalidParamSegment)),
    ])
    .run()
}

#[test]
fn normalized_conflict() {
    InsertTest(vec![
        ("/x/{foo}/bar", Ok(())),
        ("/x/{bar}/bar", Err(conflict("/x/{foo}/bar"))),
        ("/{y}/bar/baz", Ok(())),
        ("/{y}/baz/baz", Ok(())),
        ("/{z}/bar/bat", Ok(())),
        ("/{z}/bar/baz", Err(conflict("/{y}/bar/baz"))),
    ])
    .run()
}

#[test]
fn more_conflicts() {
    InsertTest(vec![
        ("/con{tact}", Ok(())),
        ("/who/are/{*you}", Ok(())),
        ("/who/foo/hello", Ok(())),
        ("/whose/{users}/{name}", Ok(())),
        ("/who/are/foo", Ok(())),
        ("/who/are/foo/bar", Ok(())),
        ("/con{nection}", Err(conflict("/con{tact}"))),
        ("/whose/{users}/{user}", Err(conflict("/whose/{users}/{name}"))),
    ])
    .run()
}

#[test]
fn catchall_static_overlap() {
    InsertTest(vec![("/bar", Ok(())), ("/bar/", Ok(())), ("/bar/{*foo}", Ok(()))]).run();

    InsertTest(vec![
        ("/foo", Ok(())),
        ("/{*bar}", Ok(())),
        ("/bar", Ok(())),
        ("/baz", Ok(())),
        ("/baz/{split}", Ok(())),
        ("/", Ok(())),
        ("/{*bar}", Err(conflict("/{*bar}"))),
        ("/{*zzz}", Err(conflict("/{*bar}"))),
        ("/{xxx}", Err(conflict("/{*bar}"))),
    ])
    .run();

    InsertTest(vec![
        ("/{*bar}", Ok(())),
        ("/bar", Ok(())),
        ("/bar/x", Ok(())),
        ("/bar_{x}", Ok(())),
        ("/bar_{x}", Err(conflict("/bar_{x}"))),
        ("/bar_{x}/y", Ok(())),
        ("/bar/{x}", Ok(())),
    ])
    .run();
}

#[test]
fn duplicate_conflict() {
    InsertTest(vec![
        ("/hey", Ok(())),
        ("/hey/users", Ok(())),
        ("/hey/user", Ok(())),
        ("/hey/user", Err(conflict("/hey/user"))),
    ])
    .run()
}

#[test]
fn invalid_param() {
    InsertTest(vec![
        ("{", Err(InsertError::InvalidParam)),
        ("}", Err(InsertError::InvalidParam)),
        ("x{y", Err(InsertError::InvalidParam)),
        ("x}", Err(InsertError::InvalidParam)),
    ])
    .run();
}

#[test]
fn escaped_param() {
    InsertTest(vec![
        ("{{", Ok(())),
        ("}}", Ok(())),
        ("xx}}", Ok(())),
        ("}}yy", Ok(())),
        ("}}yy{{}}", Ok(())),
        ("}}yy{{}}{{}}y{{", Ok(())),
        ("}}yy{{}}{{}}y{{", Err(conflict("}yy{}{}y{"))),
        ("/{{yy", Ok(())),
        ("/{yy}", Ok(())),
        ("/foo", Ok(())),
        ("/foo/{{", Ok(())),
        ("/foo/{{/{x}", Ok(())),
        ("/foo/{ba{{r}", Ok(())),
        ("/bar/{ba}}r}", Ok(())),
        ("/xxx/{x{{}}y}", Ok(())),
    ])
    .run()
}

#[test]
fn bare_catchall() {
    InsertTest(vec![("{*foo}", Ok(())), ("foo/{*bar}", Ok(()))]).run()
}
