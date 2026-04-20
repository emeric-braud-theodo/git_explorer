mod git_reader;
use git_reader::git_reader::GitReader;

fn main() {
    let git_reader = GitReader::new().expect("Cannot read repository");
    match git_reader.get_head() {
        Ok(head) => println!("{}", head),
        Err(e) => println!("Error: {}", e),
    }
}
