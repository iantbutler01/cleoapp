Skip to main content
🚀 Our new and improved config V3 is now live! See API reference for details.


Reducto API home pagelight logo

Search...
⌘K

Ask AI

Documentation
API Reference
Cookbooks
SDKs
Studio
Status
Studio
Support
Get Started
Overview
API Quickstart
Reducto CLI
Studio Quickstart
Core Functions

Upload

Parse

Extract
Split
Edit
Workflows and Pipelines

Pipelines
Chaining API Calls

Async Processing & Webhooks
Configurations
Configuration Overview

Parse Configurations

Extract Configurations

Split Configurations

Edit Configurations
Reference
Error Codes
Rate Limits
Credit Usage
Glossary
Frequently Asked Questions
Components
SpreadsheetViewer
Security and privacy
Data policies & compliance
Filing Complaints
EU data residency & processing
On-premise deployment
Hybrid VPC Deployment
Enterprise deployment options
Securing Reducto
LLM & service configuration options
On-premise changelog
Self-hosted fair queueing with Reducto
Automatic file cleanup
Air-gapped usage for billing
Get Started
API Quickstart

Copy page

Extract text, tables, and figures from documents using the Reducto API.

This guide walks you through using the Reducto API for parsing your first document within 5 mins to extract structured JSON data that can be passed to LLMs or processed further.
​
What we’re going to parse
We’ll use a financial statement PDF that contains multiple tables, headers, account summaries, and formatted text. This is the kind of complex document that’s difficult to process manually but straightforward with Reducto.
Finance Statement
Download the sample PDF to follow along.
What we want to extract:
The portfolio value table with beginning and ending values
Account information including account numbers and types
Income summary broken down by tax category
Top holdings with values and percentages
By the end of this guide, you’ll have all of this data in structured JSON that you can use in your application.
​
Prerequisites
1
Create a Reducto account

Go to studio.reducto.ai and sign up for a free account.
2
Get your API key

In the Studio sidebar, click API Keys, then Create new API key. Give it a name and copy the key.
Reducto Studio sidebar showing API Keys option
Click API Keys in the sidebar to create a new key

3
Set your API key as an environment variable

This allows the SDK to authenticate automatically without hardcoding the key in your code.
macOS / Linux
Windows (PowerShell)
export REDUCTO_API_KEY="your_api_key_here"
​
Install the SDK
Choose your language and install the Reducto SDK:
Python
Node.js
Go
pip install reductoai
​
Parse the document
Now let’s write the code to parse our financial statement. We’ll go through each part step by step.
Python
Node.js
Go
cURL
If you prefer not to use an SDK, you can call the API directly with cURL or any HTTP client.
1
Upload your document

First, upload the file to get a file reference:
curl -X POST "https://platform.reducto.ai/upload" \
  -H "Authorization: Bearer $REDUCTO_API_KEY" \
  -F "file=@finance-statement.pdf"
This returns a JSON response with a file_id:
{"file_id": "reducto://abc123def456.pdf"}
2
Parse the document

Use the file_id from the previous step as the input parameter:
curl -X POST "https://platform.reducto.ai/parse" \
  -H "Authorization: Bearer $REDUCTO_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"input": "reducto://abc123def456.pdf"}'
You can also skip the upload step if your document is already hosted at a public URL:
curl -X POST "https://platform.reducto.ai/parse" \
  -H "Authorization: Bearer $REDUCTO_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"input": "https://your-bucket.s3.amazonaws.com/finance-statement.pdf"}'
​
Understanding the response
Here’s what we got back from parsing our financial statement:
{
  "job_id": "5df31070-8d98-4caa-9a5b-c5c511a03f71",
  "duration": 11.35,
  "usage": {
    "num_pages": 3,
    "credits": 4.0
  },
  "result": {
    "chunks": [
      {
        "content": "# *** SAMPLE STATEMENT ***\nFor informational purposes only\n\nFidelity\nINVESTMENTS\n\n## Your Portfolio Value:\n\n$274,222.20\n\n|                                   | This Period   | Year-to-Date   |\n|-|-|-|\n| Beginning Portfolio Value         | $253,221.83   | $232,643.16    |\n| Additions                         | 59,269.64     | 121,433.55     |...",
        "blocks": [
          {
            "type": "Title",
            "content": "*** SAMPLE STATEMENT ***\nFor informational purposes only",
            "bbox": {"page": 1, "left": 0.351, "top": 0.029, "width": 0.296, "height": 0.057},
            "confidence": "high"
          },
          {
            "type": "Section Header",
            "content": "Your Portfolio Value:",
            "bbox": {"page": 1, "left": 0.517, "top": 0.163, "width": 0.153, "height": 0.015},
            "confidence": "high"
          },
          {
            "type": "Table",
            "content": "|                                   | This Period   | Year-to-Date   |\n|-|-|-|\n| Beginning Portfolio Value         | $253,221.83   | $232,643.16    |\n| Additions                         | 59,269.64     | 121,433.55     |\n| Subtractions                      | -45,430.74    | -98,912.58     |\n| Transaction Costs, Fees & Charges | -139.77       | -625.87        |\n| Change in Investment Value*       | 7,161.47      | 19,058.07      |\n| Ending Portfolio Value**          | $274,222.20   | $274,222.20    |",
            "bbox": {"page": 1, "left": 0.516, "top": 0.261, "width": 0.444, "height": 0.158},
            "confidence": "high"
          }
        ]
      }
    ]
  },
  "studio_link": "https://studio.reducto.ai/job/5df31070-8d98-4caa-9a5b-c5c511a03f71"
}
Key fields:
Field	What it contains
job_id	Unique identifier for this job. Use it to retrieve results later or debug in Studio.
usage.num_pages	Number of pages that were processed.
usage.credits	Credits consumed by this request.
chunks	Logical sections of the document, optimized for feeding into LLMs.
chunks[].content	The full text content of this chunk.
chunks[].blocks	Individual elements (tables, headers, text) with their types and positions.
blocks[].type	What kind of element this is: Title, Table, Header, Text, Figure, etc.
blocks[].bbox	Bounding box with normalized coordinates (0-1) showing where this element appears on the page.
studio_link	Direct link to view this job in Reducto Studio for visual debugging.
​
Customizing the output
The default settings work well for most documents, but you can customize the parsing behavior for specific use cases.
Python
Node.js
Go
cURL
curl -X POST "https://platform.reducto.ai/parse" \
  -H "Authorization: Bearer $REDUCTO_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "input": "reducto://abc123def456.pdf",
    "enhance": {
      "agentic": [{"scope": "text"}],
      "summarize_figures": true
    },
    "formatting": {
      "table_output_format": "markdown"
    },
    "settings": {
      "page_range": {"start": 1, "end": 5}
    }
  }'
What these options do:
enhance.agentic: Runs AI-powered cleanup on the specified scope. Use "text" for OCR correction on scanned documents, or "table" to improve table structure detection.
enhance.summarize_figures: Generates natural language descriptions of charts, graphs, and images. Useful for RAG pipelines where you need to search figure content.
formatting.table_output_format: Controls how tables are returned. Options are html, markdown, json, csv, or ai_json for complex tables that need AI reconstruction.
settings.page_range: Limits processing to specific pages. Useful for large documents where you only need certain sections.
For the full list of options, see the Parse configuration reference.
​
What’s next
Now that you can parse documents, explore the other Reducto endpoints:
/extract
Define a JSON schema and extract specific fields from your documents.
/split
Divide long documents into sections based on content type.
/edit
Fill PDF forms and modify DOCX documents programmatically.
/parse (async)
Process documents asynchronously with webhooks for high-volume workloads.
​
Troubleshooting
401 Unauthorized error

Tables aren't structured correctly

Content is missing or garbled

Every response includes a studio_link that opens the job in Reducto Studio. Use it to visually inspect what was extracted and debug any issues.
Was this page helpful?


Yes

No
Overview
Reducto CLI
Ask a question...

Reducto API home pagelight logo
Product

Homepage
Pricing
Playground
Company

Careers
Blog
Support
Legal

Privacy Policy
Terms of Service
x
linkedin
Powered by




