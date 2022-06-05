const Parser = require('tree-sitter');
const Natspec = require('tree-sitter-jsdoc');
const fs = require('fs');
const { Query, QueryCursor } = Parser

const parser = new Parser();
parser.setLanguage(Natspec);


function get_comment(fullComment) {
    let tree = parser.parse(fullComment)
    const queryString = `
(document
  (description)? @desc
  (tag
  )* @tag
)
`
    let query = new Query(Natspec, queryString)
    let matches = query.matches(tree.rootNode)
    let desc = null
    let commonDesc = null
    for (const match of matches) {
        for (const capture of match.captures) {
            if (capture.name === 'desc') {
                commonDesc = capture.node.text
            }
            if (capture.name === 'tag') {
                if (capture.node.children.length < 2) {
                    continue
                }
                let tagName = capture.node.children[0].text
                let tagDesc = capture.node.children[1].text
                // console.log(`${tagName} -> ${tagDesc}`)
                if (tagName === '@notice') {
                    desc = tagDesc;
                    break;
                } else if (tagName === '@dev') {
                    desc = tagDesc;
                    break;
                } else if (tagName === '@return') {
                    desc = tagDesc;
                    break;
                }
            }
        }
    }
    return desc ?? commonDesc
}

function format_comment(comment) {
    // format leading asterisk (*) in a new line
    const leadingAsteriskRe = new RegExp(/^\s*\*/gm)
    comment = comment.replace(leadingAsteriskRe, ' ')
    // remove trailing asterisks (parser issue)
    comment = comment.replace(/\*+\s*$/g, ' ')
    comment = comment.replace(/\s+/g, ' ')
    return comment.trim()
}

function format_code(code) {
    code = code.replace(/\r\n/g, '\n')
    code = code.replace(/\s+/g, ' ')
    return code
}

function replace_newline(src) {
    return src.replace(/\r\n/g, '\n')
}

const file = fs.readFileSync('data/scrl_call/test.jsonl', 'utf-8')
lines = file.split(/\r?\n/)
// newData = []
console.log(`input ${lines.length} lines`)
const fd = fs.openSync('data/scrl_call/out.jsonl', 'w')
for (const idx in lines) {
    const line = lines[idx]
    if (line === '') {
        continue
    }
    try {
        let json = JSON.parse(line)
        const [caller_code, caller_comment, callee_code, callee_comment, label] = json
        let parsed_caller_comment = get_comment(replace_newline(caller_comment))
        let parsed_callee_comment = get_comment(replace_newline(callee_comment))
        if (parsed_caller_comment === null || parsed_callee_comment === null) {
            continue
        }
        const formatted_caller_comment = format_comment(parsed_caller_comment)
        const formatted_callee_comment = format_comment(parsed_callee_comment)
        const checkLength = (s) => {
            return s.split(' ').length < 4
        }
        if (checkLength(formatted_caller_comment) || checkLength(formatted_callee_comment)) {
            continue
        }
        const data = {
            caller_code: format_code(caller_code),
            caller_comm: formatted_caller_comment,
            callee_code: format_code(callee_code),
            callee_comm: formatted_callee_comment,
            label: label
        }
        fs.writeSync(fd, JSON.stringify(data) + '\n')
        console.log(idx)
        // console.log(formatted_caller_comment)
        // console.log(formatted_callee_comment)
    } catch (e) {
        console.log(`Error when processing line:`)
        console.log(line)
        console.log(e)
        break;
    }
}

// console.log(`Write ${newData.length} samples`)
