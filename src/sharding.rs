pub fn get_index(s: &str, max: usize) -> usize {
    let mut result: usize = 0;

    for byte in s.bytes().take(8) {
        result = (result << 8) | (byte as usize);
    }

    result % max
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use uuid::Uuid;
    use super::*;

    #[test]
    fn test_get_index() {
        let test_cases = [
            ("hello", 2),
            ("world", 4),
            ("rust", 1),
            ("programming", 10),
            ("openai", 100),
        ];

        for (input, max_number) in &test_cases {
            let sharding_key = get_index(input, *max_number);
            assert!(sharding_key < *max_number, "Sharding key out of range");

            let sharding_key2 = get_index(input, *max_number);
            assert_eq!(sharding_key, sharding_key2);

        }
    }

    #[test]
    fn test_index_distribution() {
        let iterations = 100_000;
        let max_number = 10; // Number of shards
        let deviation_percent = 5;
        let expected_count = iterations / max_number as usize;
        let accept_range = expected_count * deviation_percent / 100;
        let mut key_counts = HashMap::new();

        for _i in 0..iterations {
            let input = Uuid::new_v4().to_string();
            let sharding_key = get_index(&input, max_number);
            *key_counts.entry(sharding_key).or_insert(0) += 1;
        }

        for count in key_counts.values() {
            assert!(
                *count >= expected_count - accept_range && *count <= expected_count + accept_range,
                "Sharding key distribution is not balanced"
            );
        }
    }
}