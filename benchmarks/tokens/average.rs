#[derive(Debug)]
enum StatErr {
    NoData,
}

fn average(xs: &[i64]) -> Result<i64, StatErr> {
    if xs.is_empty() {
        return Err(StatErr::NoData);
    }
    Ok(xs.iter().sum::<i64>() / xs.len() as i64)
}

fn main() {
    println!("{}", average(&[2, 4, 9]).unwrap_or(-1));
    println!("{}", average(&[]).unwrap_or(-1));
}
