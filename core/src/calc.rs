//! V 模式计算器：以 v 开头打算式（v1+2*3），候选给出结果。
//!
//! 支持 + - * / % ^、括号、小数和负号；×、÷ 也认。只做四则运算这一类，不认函数名。
//! 结果按常见写法格式化：整数不带小数点，小数最多 10 位有效数字并去掉末尾的 0。
//!
//! 只打一个数（v12345.67）时给人民币大写金额和中文数字（#92）：
//! 壹万贰仟叁佰肆拾伍元陆角柒分、一万二千三百四十五点六七。算式的结果是金额时也附上大写。

/// 算式里允许出现的字符（不含开头的 v）。
pub fn is_expr_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '+' | '-' | '*' | '/' | '%' | '^' | '.' | '(' | ')')
}

/// 计算算式；写错、没写完、除以 0 都返回 None。
pub fn evaluate(expr: &str) -> Option<f64> {
    let chars: Vec<char> = expr
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '×' => '*',
            '÷' => '/',
            other => other,
        })
        .collect();
    if chars.is_empty() {
        return None;
    }
    let mut parser = Parser { chars, pos: 0 };
    let value = parser.expr()?;
    if parser.pos != parser.chars.len() || !value.is_finite() {
        return None;
    }
    Some(value)
}

/// 把结果写成人看的样子：7、0.5、-3.333333333、1e+21 这类太大的用科学计数法。
pub fn format(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    if value.abs() >= 1e15 || value.abs() < 1e-9 {
        return format!("{value:e}");
    }
    let rounded = (value * 1e10).round() / 1e10;
    if rounded.fract() == 0.0 {
        return format!("{}", rounded as i64);
    }
    let digits = 10usize.saturating_sub(rounded.abs().log10().floor().max(0.0) as usize + 1);
    let text = format!("{rounded:.digits$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

/// 候选：算得出来时是「结果」和「算式=结果」；还没写完时只给算式本身。
/// 只打一个数时是大写金额、中文数字和数字本身。
pub fn candidates(expr: &str) -> Vec<String> {
    if is_plain_number(expr) {
        let mut out: Vec<String> = [rmb_upper(expr), chinese_number(expr)].into_iter().flatten().collect();
        out.push(expr.to_string());
        return out;
    }
    match evaluate(expr) {
        Some(value) => {
            let result = format(value);
            let mut out = vec![result.clone(), format!("{expr}={result}")];
            out.extend(rmb_upper(&result));
            out
        }
        None if !expr.is_empty() => vec![expr.to_string()],
        None => Vec::new(),
    }
}

/// 只有数字、最多一个小数点（12345、0.5、3.14）。
fn is_plain_number(text: &str) -> bool {
    let mut parts = text.split('.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next();
    parts.next().is_none()
        && !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// 整数部分最多到千亿（12 位）。
const MAX_INT_DIGITS: usize = 12;

/// 拆成整数部分（去掉开头的 0）和小数部分；不是普通的非负数、整数部分太长时 None。
fn split_number(text: &str) -> Option<(u64, &str)> {
    if !is_plain_number(text) {
        return None;
    }
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let int = int.trim_start_matches('0');
    if int.len() > MAX_INT_DIGITS {
        return None;
    }
    Some((int.parse().unwrap_or(0), frac))
}

/// 整数写成汉字，四位一节（万、亿），节内和节间的 0 只读一个「零」。
fn integer_words(n: u64, digits: &[&str; 10], units: &[&str; 4]) -> String {
    if n == 0 {
        return digits[0].to_string();
    }
    let mut sections = Vec::new();
    let mut rest = n;
    while rest > 0 {
        sections.push((rest % 10000) as usize);
        rest /= 10000;
    }
    let mut out = String::new();
    let mut gap = false;
    for (i, &section) in sections.iter().enumerate().rev() {
        if section == 0 {
            gap = !out.is_empty();
            continue;
        }
        // 前面有字、这一节不满四位或中间空了一节：补「零」（10001 一万零一、1亿零1万）
        if !out.is_empty() && (gap || section < 1000) {
            out.push_str(digits[0]);
        }
        let mut zero = false;
        for pos in (0..4).rev() {
            let d = section / 10usize.pow(pos as u32) % 10;
            if d == 0 {
                zero = true;
                continue;
            }
            if zero && out_has_digit_in_section(section, pos) {
                out.push_str(digits[0]);
            }
            zero = false;
            out.push_str(digits[d]);
            out.push_str(units[pos]);
        }
        out.push_str(["", "万", "亿"][i]);
        gap = false;
    }
    out
}

/// 这一节里 pos 位前面（更高位）有没有非 0 的数字：有才需要补「零」（1010 一千零一十）。
fn out_has_digit_in_section(section: usize, pos: usize) -> bool {
    section / 10usize.pow(pos as u32 + 1) > 0
}

/// 大写金额用的数字
const UPPER_DIGITS: [&str; 10] = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"];
/// 大写金额的节内单位（个十百千）
const UPPER_UNITS: [&str; 4] = ["", "拾", "佰", "仟"];
/// 中文小写用的数字
const LOWER_DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
/// 中文小写的节内单位（个十百千）
const LOWER_UNITS: [&str; 4] = ["", "十", "百", "千"];

/// 人民币大写金额：12345.67 → 壹万贰仟叁佰肆拾伍元陆角柒分，100 → 壹佰元整。
/// 小数超过两位（分以下）、整数部分超过千亿时 None。
pub fn rmb_upper(text: &str) -> Option<String> {
    let (int, frac) = split_number(text)?;
    if frac.len() > 2 {
        return None;
    }
    let digit = |i: usize| frac.as_bytes().get(i).map_or(0, |b| (b - b'0') as usize);
    let (jiao, fen) = (digit(0), digit(1));
    let mut out = String::new();
    if int > 0 {
        out.push_str(&integer_words(int, &UPPER_DIGITS, &UPPER_UNITS));
        out.push('元');
    }
    if jiao == 0 && fen == 0 {
        if int == 0 {
            out.push_str("零元");
        }
        out.push('整');
        return Some(out);
    }
    if jiao > 0 {
        out.push_str(UPPER_DIGITS[jiao]);
        out.push('角');
    } else if int > 0 {
        // 有元、角是 0、分不是 0：123.05 → 壹佰贰拾叁元零伍分
        out.push('零');
    }
    if fen > 0 {
        out.push_str(UPPER_DIGITS[fen]);
        out.push('分');
    }
    Some(out)
}

/// 中文数字：12345.67 → 一万二千三百四十五点六七，10 → 十，15 → 十五。
pub fn chinese_number(text: &str) -> Option<String> {
    let (int, frac) = split_number(text)?;
    let mut out = integer_words(int, &LOWER_DIGITS, &LOWER_UNITS);
    // 一十、一十五、一十万读作十、十五、十万
    if let Some(rest) = out.strip_prefix("一十") {
        out = format!("十{rest}");
    }
    if !frac.is_empty() {
        out.push('点');
        for b in frac.bytes() {
            out.push_str(LOWER_DIGITS[(b - b'0') as usize]);
        }
    }
    Some(out)
}

/// 递归下降的算式解析器：字符流和当前读到的位置
struct Parser {
    /// 整条算式拆成的字符。
    chars: Vec<char>,
    /// 当前读到的字符下标。
    pos: usize,
}

impl Parser {
    /// 看下一个字符，不前进
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// 下一个字符是这个就吃掉并返回 true
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.eat('+') {
                value += self.term()?;
            } else if self.eat('-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    // term := unary (('*' | '/' | '%') unary)*
    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.eat('*') {
                value *= self.unary()?;
            } else if self.eat('/') {
                let rhs = self.unary()?;
                if rhs == 0.0 {
                    return None;
                }
                value /= rhs;
            } else if self.eat('%') {
                let rhs = self.unary()?;
                if rhs == 0.0 {
                    return None;
                }
                value %= rhs;
            } else {
                return Some(value);
            }
        }
    }

    // unary := ('-' | '+') unary | power
    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    // power := atom ('^' unary)?   右结合：2^3^2 = 2^9
    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat('^') {
            let exp = self.unary()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    // atom := number | '(' expr ')'
    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let value = self.expr()?;
            return self.eat(')').then_some(value);
        }
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        if start == self.pos {
            return None;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 算完再格式化，方便断言
    fn calc(expr: &str) -> Option<String> {
        evaluate(expr).map(format)
    }

    /// 优先级、括号、减法和除法左结合、乘方右结合、取模都对，×÷ 当 * /
    #[test]
    fn precedence() {
        assert_eq!(calc("1+2*3").as_deref(), Some("7"));
        assert_eq!(calc("(1+2)*3").as_deref(), Some("9"));
        assert_eq!(calc("10-4-3").as_deref(), Some("3"));
        assert_eq!(calc("2^3^2").as_deref(), Some("512"));
        assert_eq!(calc("-2^2").as_deref(), Some("-4"));
        assert_eq!(calc("7%3").as_deref(), Some("1"));
        assert_eq!(calc("3×4÷2").as_deref(), Some("6"));
    }

    /// 小数结果去掉浮点误差和末尾的 0
    #[test]
    fn decimals() {
        assert_eq!(calc("1/2").as_deref(), Some("0.5"));
        assert_eq!(calc("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(calc("10/3").as_deref(), Some("3.333333333"));
        assert_eq!(calc("1.5*2").as_deref(), Some("3"));
    }

    /// 没写完、括号不配对、除以 0、非数字都给 None
    #[test]
    fn invalid() {
        for expr in ["", "1+", "(1+2", "1/0", "1..2", "abc", "1+2)"] {
            assert_eq!(calc(expr), None, "{expr}");
        }
    }

    /// 大写金额：整、复零的补法、角分为 0 的写法，小数超两位和超长的不给
    #[test]
    fn rmb_uppercase() {
        let cases = [
            ("12345", "壹万贰仟叁佰肆拾伍元整"),
            ("12345.67", "壹万贰仟叁佰肆拾伍元陆角柒分"),
            ("100", "壹佰元整"),
            ("123.05", "壹佰贰拾叁元零伍分"),
            ("123.5", "壹佰贰拾叁元伍角"),
            ("0", "零元整"),
            ("0.05", "伍分"),
            ("0.50", "伍角"),
            ("10", "壹拾元整"),
            ("10001", "壹万零壹元整"),
            ("1010", "壹仟零壹拾元整"),
            ("100100", "壹拾万零壹佰元整"),
            ("1000000.10", "壹佰万元壹角"),
            ("100000001", "壹亿零壹元整"),
            ("100010000", "壹亿零壹万元整"),
            ("200000000000", "贰仟亿元整"),
            ("007", "柒元整"),
        ];
        for (input, want) in cases {
            assert_eq!(rmb_upper(input).as_deref(), Some(want), "{input}");
        }
        for input in ["1.234", "1000000000000", "-1", "1+2", "", ".5", "5."] {
            assert_eq!(rmb_upper(input), None, "{input}");
        }
    }

    /// 中文小写：一十读作十，小数逐位读
    #[test]
    fn chinese_lowercase() {
        let cases = [
            ("12345.67", "一万二千三百四十五点六七"),
            ("10", "十"),
            ("15", "十五"),
            ("110", "一百一十"),
            ("100000", "十万"),
            ("10001", "一万零一"),
            ("0", "零"),
            ("3.14", "三点一四"),
            ("2026", "二千零二十六"),
        ];
        for (input, want) in cases {
            assert_eq!(chinese_number(input).as_deref(), Some(want), "{input}");
        }
    }

    /// 候选顺序：纯数字给大写、中文数字和原数，算式给结果和算式=结果，结果是金额时再加大写
    #[test]
    fn candidate_list() {
        assert_eq!(
            candidates("12345.67"),
            vec!["壹万贰仟叁佰肆拾伍元陆角柒分", "一万二千三百四十五点六七", "12345.67"]
        );
        assert_eq!(candidates("3.14159"), vec!["三点一四一五九", "3.14159"]);
        assert_eq!(candidates("12*3.5"), vec!["42", "12*3.5=42", "肆拾贰元整"]);
        assert_eq!(candidates("10/3"), vec!["3.333333333", "10/3=3.333333333"]);
        assert_eq!(candidates("1+2*3"), vec!["7", "1+2*3=7", "柒元整"]);
        assert_eq!(candidates("1+"), vec!["1+"]);
        assert!(candidates("").is_empty());
    }
}
