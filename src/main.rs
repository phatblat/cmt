fn greeting() -> String {
    "Hello, world!".to_string()
}

fn main() {
    println!("{}", greeting());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_says_hello() {
        assert_eq!(greeting(), "Hello, world!");
    }
}
