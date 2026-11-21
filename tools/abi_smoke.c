/* Flype C ABI 冒烟测试：模拟 fcitx5 插件的调用序列。
 *
 * 覆盖：引擎创建 -> 输入 -> 取候选 -> 记录选词 -> 设置上下文 -> 预测下一词
 *      -> 用户词增删 -> 用户词库持久化。
 */
#include <stdio.h>
#include <string.h>
#include "flype_core.h"

// 失败的条数，最后按它决定退出码
static int failures = 0;

// 打一行 ok 或 FAIL 到标准输出，失败时累加计数
static void check(int cond, const char *what) {
    if (!cond) {
        failures++;
        printf("FAIL %s", what);
        putchar(10);
    } else {
        printf("ok   %s", what);
        putchar(10);
    }
}

// 候选列表里有没有文字一模一样的这一条
static int has_candidate(FlypeCandidates *c, const char *text) {
    size_t n = flype_candidates_len(c);
    for (size_t i = 0; i < n; i++) {
        const char *t = flype_candidates_text(c, i);
        if (t && strcmp(t, text) == 0) return 1;
    }
    return 0;
}

// 按顺序跑一遍插件会用的 ABI 调用，参数是词库、二元模型、简繁表路径，返回码表示有没有失败
int main(int argc, char **argv) {
    const char *lexicon = argc > 1 ? argv[1] : "/tmp/flype_abi.tsv";
    const char *bigram = argc > 2 ? argv[2] : "";
    FlypeEngineHandle *e = flype_engine_new(lexicon, bigram);
    if (!e) {
        printf("engine_new failed: %s", flype_last_error());
        putchar(10);
        return 1;
    }

    FlypeCandidates *c = flype_engine_input(e, "nihao");
    check(c && has_candidate(c, "你好"), "exact: nihao -> 你好");
    flype_candidates_free(c);

    c = flype_engine_input(e, "nih");
    check(c && has_candidate(c, "你好"), "partial: nih -> 你好");
    flype_candidates_free(c);

    c = flype_engine_input(e, "hello");
    check(c && has_candidate(c, "hello"), "english: hello -> hello");
    flype_candidates_free(c);

    /* 句首词候选：consumed 小于输入长度，剩下的部分留给下一轮 */
    c = flype_engine_input(e, "nihaozhongguo");
    {
        int found = 0;
        size_t n = flype_candidates_len(c);
        for (size_t i = 0; c && i < n; i++) {
            const char *t = flype_candidates_text(c, i);
            if (t && strcmp(t, "你好") == 0 && flype_candidates_consumed(c, i) == 5) found = 1;
        }
        check(found, "prefix: nihaozhongguo 有 你好(用掉 5 字节)");
        check(c && flype_candidates_consumed(c, 0) == 13, "整句候选用掉全部输入");
        const char *py = c ? flype_candidates_pinyin(c, 0) : NULL;
        check(py && strcmp(py, "ni hao zhong guo") == 0, "candidates_pinyin");
    }
    flype_candidates_free(c);

    /* 按错键：zhonggup（o 旁边的 p） */
    c = flype_engine_input(e, "zhonggup");
    check(c && has_candidate(c, "中国"), "typo: zhonggup -> 中国");
    flype_candidates_free(c);
    check(flype_engine_set_typo_correction(e, 0) == 0, "set_typo_correction");
    c = flype_engine_input(e, "zhonggup");
    check(c && !has_candidate(c, "中国"), "纠错关闭后 zhonggup 不出 中国");
    flype_candidates_free(c);
    flype_engine_set_typo_correction(e, 1);
    check(flype_engine_set_max_candidates(e, 5) == 0, "set_max_candidates");

    check(flype_engine_record_selection(e, "你好") == 0, "record_selection");
    check(flype_engine_set_context(e, "今天") == 0, "set_context");

    c = flype_engine_input(e, "tian");
    check(c && has_candidate(c, "天"), "context: tian -> 天");
    flype_candidates_free(c);

    c = flype_engine_predict_next(e, "今天", 3);
    check(c && flype_candidates_len(c) > 0, "predict_next 有结果");
    flype_candidates_free(c);

    /* 领域模型：设两次是换掉不是叠着混；混上以后联想照样按通用模型；设错的路径不动原来的 */
    if (*bigram) {
        check(flype_engine_set_domain_lm(e, bigram) == 0, "set_domain_lm");
        check(flype_engine_set_domain_lm(e, bigram) == 0, "set_domain_lm 再设一次");
        c = flype_engine_predict_next(e, "今天", 3);
        check(c && flype_candidates_len(c) > 0, "混上领域模型后 predict_next 有结果");
        flype_candidates_free(c);
    }
    check(flype_engine_set_domain_lm(e, "/nonexistent/words.domain.lm") == -1, "set_domain_lm 文件不在返回 -1");
    check(flype_engine_set_domain_lm(e, NULL) == 0, "set_domain_lm(NULL) 去掉");
    check(flype_nnlm_check("/nonexistent/nlrgpt2_nnlm.bin") == -1, "nnlm_check 文件不在返回 -1");
    check(flype_nnlm_check(lexicon) == -1, "nnlm_check 词库不是模型");

    check(flype_engine_add_user_word(e, "wo wo te", "我特") == 0, "add_user_word");
    c = flype_engine_input(e, "wowote");
    check(c && has_candidate(c, "我特"), "user word 可输入");
    flype_candidates_free(c);

    check(flype_engine_set_fuzzy(e, 1) == 0, "set_fuzzy");
    c = flype_engine_input(e, "zidao");
    check(c && has_candidate(c, "知道"), "fuzzy: zidao -> 知道");
    flype_candidates_free(c);

    check(flype_engine_save_user_dict(e, "/tmp/flype_abi_user.tsv") == 0, "save_user_dict");
    check(flype_engine_remove_user_word(e, "我特") == 1, "remove_user_word");
    check(flype_engine_load_user_dict(e, "/tmp/flype_abi_user.tsv") >= 1, "load_user_dict");
    /* 后台存盘（1.11）：交出去就返回，等写完以后读得回来；写不了的只报一次错 */
    remove("/tmp/flype_abi_user_async.tsv");
    check(flype_engine_save_user_dict_async(e, "/tmp/flype_abi_user_async.tsv") == 0,
          "save_user_dict_async");
    check(flype_engine_save_history_async(e, "/tmp/flype_abi_history_async.tsv") == 0,
          "save_history_async");
    check(flype_engine_save_wait(e, -1) == 0, "save_wait");
    check(flype_engine_load_user_dict(e, "/tmp/flype_abi_user_async.tsv") >= 1,
          "async user dict readable");
    check(flype_engine_save_history_async(e, "/tmp/flype_abi_no_such_dir/h.tsv") == 0,
          "save_history_async queues");
    check(flype_engine_save_wait(e, -1) == -1, "save_wait reports a failed write");
    check(flype_engine_save_wait(e, 0) == 0, "save_wait reports it once");
    {
        FILE *custom = fopen("/tmp/flype_abi_custom.tsv", "w");
        check(custom != NULL, "open custom.tsv");
        fputs("ni hao\t妮好\nnot pinyin zz\t坏\n", custom);
        fclose(custom);
        check(flype_engine_load_custom_words(e, "/tmp/flype_abi_custom.tsv") == 1,
              "load_custom_words");
        check(flype_engine_load_custom_words(e, "/tmp/flype_abi_no_such.tsv") == 0,
              "load_custom_words missing file clears");
        remove("/tmp/flype_abi_custom.tsv");
    }

    /* 自动学习：同一个词选 3 次后应当能直接打出来 */
    for (int i = 0; i < 3; i++) {
        flype_engine_record_selection_ex(e, "阿特", "a te");
    }
    c = flype_engine_input(e, "ate");
    check(c && has_candidate(c, "阿特"), "auto learn: ate -> 阿特");
    flype_candidates_free(c);

    c = flype_symbol_search("juhao", 3);
    check(c && flype_candidates_len(c) > 0 &&
              strcmp(flype_candidates_text(c, 0), "。") == 0,
          "symbol: juhao -> 。");
    flype_candidates_free(c);
    {
        const char *p = flype_fullwidth_punct(',');
        check(p && strcmp(p, "，") == 0, "punct: comma -> ，");
    }

    /* 输入会话（ABI 1.3）：字母进编辑区、退格、切换输入法时上屏。
     * 不带引擎（词库还没载入好时就是这样）：编辑区是原始输入 */
    {
        FlypeSession *s = flype_session_new(NULL, NULL);
        check(s != NULL, "session_new");
        const FlypeFrame *f = NULL;
        const char *keys = "nihaoma";
        int letters_ok = 1;
        for (const char *k = keys; *k; ++k) {
            FlypeKey key = {sizeof(FlypeKey), (uint32_t)*k, 0, 0, 0};
            f = flype_session_key(s, &key);
            letters_ok = letters_ok && f && f->handled && f->size == sizeof(FlypeFrame);
            FlypeKey up = key;
            up.release = 1;
            letters_ok = letters_ok && !flype_session_key(s, &up)->handled;
        }
        check(letters_ok, "session: letters handled, releases not, frame size matches header");
        /* f 指向会话自己的 frame：上面的松开键已经覆盖了它，这里看的是松开后的编辑区 */
        check(f && strcmp(f->preedit, "nihaoma") == 0 && f->commit == NULL, "session: preedit nihaoma");
        f = flype_session_choose(s, 0);
        check(f && !f->handled && f->commit == NULL, "session: choose without engine");
        FlypeKey bs = {sizeof(FlypeKey), 0xff08, 0, 0, 0};
        f = flype_session_key(s, &bs);
        check(f && f->handled && strcmp(f->preedit, "nihaom") == 0, "session: backspace");
        /* ABI 1.6：没有候选时空格上屏原始输入 */
        FlypeKey space = {sizeof(FlypeKey), ' ', 0, 0, 0};
        f = flype_session_key(s, &space);
        check(f && f->handled && f->commit && strcmp(f->commit, "nihaom") == 0 && f->preedit[0] == 0,
              "session: space commits raw input without candidates");
        for (const char *k = "ab"; *k; ++k) {
            FlypeKey key = {sizeof(FlypeKey), (uint32_t)*k, 0, 0, 0};
            flype_session_key(s, &key);
        }
        f = flype_session_reset(s, 1);
        check(f && f->commit && strcmp(f->commit, "ab") == 0 && f->preedit[0] == 0,
              "session: reset commits raw input");
        flype_session_free(s);
    }

    /* 输入会话带引擎（ABI 1.4）：候选列表和分音节 */
    {
        FlypeSessionConfig config = {sizeof(FlypeSessionConfig), 9, 1, 0, 0};
        FlypeSession *s = flype_session_new(NULL, &config);
        for (const char *k = "nihaozhongguo"; *k; ++k) {
            FlypeKey key = {sizeof(FlypeKey), (uint32_t)*k, 0, 0, 0};
            flype_session_key(s, &key);
        }
        check(flype_session_candidate_total(s) == 0, "session: no engine, no candidates");
        flype_session_set_engine(s, e);
        const FlypeFrame *f = flype_session_frame(s);
        check(f && !f->handled && strcmp(f->preedit, "ni'hao'zhong'guo") == 0,
              "session: preedit segmented");
        check(flype_session_candidate_total(s) > 0, "session: candidates with engine");
        FlypeCandidateInfo info;
        memset(&info, 0, sizeof info);
        info.size = sizeof info;
        check(flype_session_candidate_info(s, 0, &info) == 0 && strcmp(info.text, "你好中国") == 0 &&
                  strcmp(info.pinyin, "ni hao zhong guo") == 0 && info.consumed == 13 &&
                  info.kind == FLYPE_CAND_WORD && strcmp(info.commit, "你好中国") == 0 &&
                  info.emoji[0] == 0 && info.size == sizeof info,
              "session: candidate info");
        check(flype_session_candidate_info(s, flype_session_candidate_total(s), &info) == -1,
              "session: candidate info out of range");
        /* ABI 1.5：选句首词进已选栈，剩下的接着选；选完整句上屏 */
        size_t nihao = flype_session_candidate_total(s);
        for (size_t i = 0; i < flype_session_candidate_total(s); ++i) {
            if (flype_session_candidate_info(s, i, &info) == 0 && strcmp(info.text, "你好") == 0) {
                nihao = i;
                break;
            }
        }
        check(nihao < flype_session_candidate_total(s) && info.consumed == 5, "session: 你好 candidate");
        f = flype_session_choose(s, nihao);
        check(f && f->handled && f->commit == NULL && strcmp(f->preedit, "你好zhong'guo") == 0,
              "session: choose sentence-start word");
        check(flype_session_candidate_info(s, 0, &info) == 0 && strcmp(info.text, "中国") == 0 &&
                  info.consumed == 8,
              "session: candidates follow selection");
        f = flype_session_choose(s, 0);
        check(f && f->handled && f->commit && strcmp(f->commit, "你好中国") == 0 && f->preedit[0] == 0,
              "session: choose commits whole sentence");
        for (const char *k = "nihaozh"; *k; ++k) {
            FlypeKey key = {sizeof(FlypeKey), (uint32_t)*k, 0, 0, 0};
            flype_session_key(s, &key);
        }
        FlypeKey enter = {sizeof(FlypeKey), 0xff0d, 0, 0, 0};
        f = flype_session_key(s, &enter);
        /* 回车上屏的原始字母串不记选词（终端里的密码提示没有密码框标记） */
        check(f && f->commit && strcmp(f->commit, "nihaozh") == 0 && !f->learned,
              "session: return commits raw letters (not learned)");
        /* ABI 1.6：显示的这一页、点候选、上屏的是哪个候选、候选框状态 */
        for (const char *k = "nihao"; *k; ++k) {
            FlypeKey key = {sizeof(FlypeKey), (uint32_t)*k, 0, 0, 0};
            flype_session_key(s, &key);
        }
        f = flype_session_frame(s);
        const size_t shown = f ? f->cand_count : 0;
        check(shown > 0 && shown <= 9 && f->cand_cursor == 0 && f->panel == 0 && !f->has_prev,
              "session: frame shows a page");
        check(flype_session_candidate(s, shown) == NULL, "session: candidate out of range");
        char first[64] = {0};
        const char *text = flype_session_candidate(s, 0);
        check(text != NULL, "session: candidate text");
        if (text) {
            strncpy(first, text, sizeof first - 1);
        }
        FlypeSessionView view;
        memset(&view, 0, sizeof view);
        view.size = sizeof view;
        check(flype_session_view(s, &view) == 0 && view.page == 0 && view.cursor == 0 &&
                  !view.expanded && view.size == sizeof view,
              "session: view");
        check(!flype_session_select(s, shown)->handled, "session: select out of range");
        f = flype_session_select(s, 0);
        check(f && f->handled && f->commit && strcmp(f->commit, first) == 0, "session: select commits");
        check(f->learned && f->mode == FLYPE_MODE_CHINESE, "session: 1.10 frame fields");
        /* ABI 1.10：没在打字时标点由会话换成中文标点 */
        FlypeKey comma = {sizeof(FlypeKey), ',', 0, 0, 0};
        f = flype_session_key(s, &comma);
        check(f && f->handled && f->commit && strcmp(f->commit, "，") == 0 && !f->learned,
              "session: punctuation");
        /* ABI 1.10：设置里的切换键由外壳认出来，带上动作交给会话 */
        FlypeKey toggle = {sizeof(FlypeKey), ' ', FLYPE_MOD_CTRL | FLYPE_MOD_SHIFT, 0, 0,
                             FLYPE_KEY_ACTION_TOGGLE_MODE};
        f = flype_session_key(s, &toggle);
        check(f && f->handled && f->mode_changed && f->mode == FLYPE_MODE_ENGLISH, "session: toggle key");
        flype_session_set_mode(s, FLYPE_MODE_CHINESE);
        flype_session_set_engine(s, NULL);
        check(flype_session_candidate_total(s) == 0, "session: engine unset");
        flype_session_free(s);
    }

    /* ABI 1.10：自定义词的编码整理 */
    {
        const char *code = flype_custom_code("Ni H\xC7\x8Eo", 1);
        check(code && strcmp(code, "ni hao") == 0, "custom code: pinyin");
        code = flype_custom_code("Mail", 0);
        check(code && strcmp(code, "mail") == 0, "custom code: letters");
        check(flype_custom_code("Mail", 1) == NULL, "custom code: pinyin only");
        const char lex[] = "ni\t\xE4\xBD\xA0\t1\nhao\t\xE5\xA5\xBD\t1\n";
        code = flype_guess_pinyin(lex, sizeof lex - 1, "\xE4\xBD\xA0\xE5\xA5\xBD");
        check(code && strcmp(code, "ni hao") == 0, "custom code: guess pinyin");
    }

    if (flype_abi_version() / 1000 != FLYPE_ABI_VERSION_MAJOR) {
        fprintf(stderr, "ABI 主版本不一致：库 %d，头文件 %d\n", flype_abi_version(),
                FLYPE_ABI_VERSION_MAJOR);
        return 1;
    }
    printf("abi=%d ", flype_abi_version());
    printf("version=%s", flype_version());
    putchar(10);
    flype_engine_free(e);

    printf(failures ? "ABI SMOKE FAILED" : "ABI SMOKE OK");
    putchar(10);
    return failures ? 1 : 0;
}