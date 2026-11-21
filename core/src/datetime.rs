//! 日期、时间候选：打 riqi / shijian / xingqi（或英文 date / time）时，在首选词后面
//! 给出当前日期、时间的几种写法。
//!
//! 只认完整拼音，不认 rq、sj 这类简拼——它们太常见，平时打别的词会被打扰。
//! 当前时间由调用方（输入法插件）按本地时区传进来，这里不读系统时钟，方便测试。

/// 本地时间。weekday：0 = 星期日，1 = 星期一 …… 6 = 星期六。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// 公历年（如 2026）。
    pub year: i32,
    /// 月，1–12。
    pub month: u32,
    /// 日，1–31。
    pub day: u32,
    /// 小时，24 小时制（0–23）。
    pub hour: u32,
    /// 分，0–59。
    pub minute: u32,
    /// 秒，0–59。
    pub second: u32,
    /// 星期几：0 = 星期日，1 = 星期一 …… 6 = 星期六。
    pub weekday: u32,
}

/// weekday 下标对应的汉字「日」到「六」
const WEEKDAYS: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

/// 触发词本身是哪个词（riqi -> 日期）：日期时间的写法挂在它后面，它要排第一。英文触发词没有。
pub fn trigger_word(key: &str) -> Option<&'static str> {
    match key {
        "riqi" | "rq" => Some("日期"),
        "shijian" | "sj" => Some("时间"),
        "xingqi" => Some("星期"),
        _ => None,
    }
}

/// 触发词（拼音或英文，小写、不带隔音符）对应的候选；不是触发词时返回空。
pub fn candidates(key: &str, now: &LocalTime) -> Vec<String> {
    let key: String = key
        .chars()
        .filter(|c| !matches!(c, '\'' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    let LocalTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: s,
        weekday,
    } = *now;
    let week = WEEKDAYS[(weekday % 7) as usize];
    match key.as_str() {
        "riqi" | "rq" | "date" => vec![
            format!("{y}年{mo}月{d}日"),
            format!("{y}-{mo:02}-{d:02}"),
            format!("{y}/{mo:02}/{d:02}"),
            format!("{y}.{mo:02}.{d:02}"),
            format!("{y}{mo:02}{d:02}"),
            format!("{mo}月{d}日"),
            format!("{y}年{mo}月{d}日 星期{week}"),
        ],
        "shijian" | "sj" | "time" => vec![
            format!("{h:02}:{mi:02}"),
            format!("{h:02}:{mi:02}:{s:02}"),
            format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}"),
            format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"),
            format!("{h}时{mi}分"),
            format!("{y}年{mo}月{d}日 {h:02}:{mi:02}"),
        ],
        "xingqi" => vec![format!("星期{week}"), format!("周{week}")],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定的本地时间：2026-11-10 09:05:01 星期二
    fn now() -> LocalTime {
        LocalTime {
            year: 2026,
            month: 11,
            day: 10,
            hour: 9,
            minute: 5,
            second: 1,
            weekday: 2,
        }
    }

    /// riqi、date、ri'qi 给同一组日期写法，第一个是「年月日」，最后还带星期
    #[test]
    fn date_formats() {
        let c = candidates("riqi", &now());
        assert_eq!(c[0], "2026年11月10日");
        assert!(c.contains(&"2026-11-10".to_string()));
        assert!(c.contains(&"2026/11/10".to_string()));
        assert!(c.contains(&"2026.11.10".to_string()));
        assert!(c.contains(&"20261110".to_string()));
        assert!(c.contains(&"2026年11月10日 星期二".to_string()));
        assert_eq!(candidates("date", &now()), c);
        assert_eq!(candidates("ri'qi", &now()), c);
    }

    /// shijian、TIME 给同一组时间写法，第一个是 24 小时制的时:分
    #[test]
    fn time_formats() {
        let c = candidates("shijian", &now());
        assert_eq!(c[0], "09:05");
        assert!(c.contains(&"09:05:01".to_string()));
        assert!(c.contains(&"2026-11-10 09:05".to_string()));
        assert!(c.contains(&"9时5分".to_string()));
        assert_eq!(candidates("TIME", &now()), c);
    }

    /// xingqi 只给星期X、周X 两种，0 是星期日而不是星期七
    #[test]
    fn weekday() {
        assert_eq!(candidates("xingqi", &now()), vec!["星期二", "周二"]);
        let sunday = LocalTime {
            weekday: 0,
            ..now()
        };
        assert_eq!(candidates("xingqi", &sunday)[0], "星期日");
    }

    /// rq、sj 照样触发，xq、riq 这些没打完的不出日期时间候选
    #[test]
    fn abbreviations_do_not_trigger() {
        assert_eq!(candidates("rq", &now()), candidates("riqi", &now()));
        assert_eq!(candidates("sj", &now()), candidates("shijian", &now()));
        for key in ["xq", "riq", "shij", "dat", ""] {
            assert!(candidates(key, &now()).is_empty(), "{key} 不该出日期时间");
        }
    }
}
