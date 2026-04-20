use crate::git_reader::git_reader::GitReader;
use std::io::{self, Write};

pub struct CLI {
    reader: GitReader,
}

impl CLI {
    pub fn new() -> Result<Self, git2::Error> {
        Ok(Self {
            reader: GitReader::new()?,
        })
    }
    pub fn listen(&self) -> Result<(), git2::Error> {
        loop {
            print!("neurogit> ");
            io::stdout().flush().unwrap();

            let mut input = String::new();

            io::stdin().read_line(&mut input).expect("Reading error");
            let command = input.trim();

            match command {
                "exit" | "quit" => break,
                "head" => {
                    if let Ok(h) = self.reader.get_head() {
                        println!("HEAD is at: {}", h);
                    }
                }
                "c list" => {
                    if let Ok(l) = self.reader.list_commits() {
                        println!("{}", l.join("\n"));
                    }
                }
                "head diff" => {
                    if let Ok(diff) = self
                        .reader
                        .get_commit_diff(&self.reader.get_repo().head()?.peel_to_commit()?)
                    {
                        println!("{}", diff.to_string())
                    }
                }
                _ => println!("Unknown command : {}", command),
            }
        }
        Ok(())
    }
}
