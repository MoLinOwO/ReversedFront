const path = require('path');
const TerserPlugin = require('terser-webpack-plugin');

module.exports = {
    // 確保無論從哪個工作目錄執行 webpack，都以本目錄 (web/mod) 為根
    context: __dirname,
    mode: 'production',
    entry: './js/index.js',
    output: {
        filename: 'main.bundle.js',
        path: path.resolve(__dirname, 'js'),
    },
    optimization: {
        minimize: true,
        minimizer: [
            new TerserPlugin({
                terserOptions: {
                    compress: {
                        drop_console: true,
                        drop_debugger: true,
                        pure_funcs: ['console.log', 'console.debug'],
                    },
                    mangle: {
                        toplevel: true,
                    },
                    format: {
                        comments: false,
                    },
                },
                extractComments: false,
            }),
        ],
    },
};
