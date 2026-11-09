enum BankErr {
    NoFunds(i64),
    NoAccount,
}

fn withdraw(a: &mut [i64], i: usize, n: i64) -> Result<(), BankErr> {
    if i >= a.len() {
        return Err(BankErr::NoAccount);
    }
    if a[i] < n {
        return Err(BankErr::NoFunds(n - a[i]));
    }
    a[i] -= n;
    Ok(())
}

fn attempt(a: &mut [i64], i: usize, n: i64) {
    match withdraw(a, i, n) {
        Ok(()) => println!("ok {}", a[i]),
        Err(BankErr::NoFunds(short)) => println!("short by {}", short),
        Err(BankErr::NoAccount) => println!("no account"),
    }
}

fn main() {
    let mut a = vec![100, 5];
    attempt(&mut a, 0, 30);
    attempt(&mut a, 1, 30);
    attempt(&mut a, 7, 1);
}
