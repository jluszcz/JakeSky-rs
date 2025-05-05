terraform {
  backend "s3" {}
}

# Sourced from environment variables named TF_VAR_${VAR_NAME}
variable "aws_acct_id" {}

variable "jakesky_api_key" {}

variable "jakesky_geocodio_key" {}

variable "jakesky_skill_id" {}

variable "jakesky_latitude" {}

variable "jakesky_longitude" {}

variable "code_bucket" {}

variable "aws_region" {
  type    = string
  default = "us-east-1"
}

provider "aws" {
  region = var.aws_region
}

resource "aws_cloudwatch_event_rule" "jakesky_schedule" {
  name                = "jakesky-schedule"
  description         = "Run jakesky periodically to keep the Lambda warmed"
  schedule_expression = "cron(0/5 11_13 ? * * *)"
}

resource "aws_lambda_permission" "jakesky_allow_cloudwatch" {
  statement_id  = "Jakesky-AllowExecutionFromCloudWatch"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.jakesky.arn
  principal     = "events.amazonaws.com"
  source_arn    = aws_cloudwatch_event_rule.jakesky_schedule.arn
}

data "aws_iam_policy_document" "jakesky_role_policy_document" {
  statement {
    actions   = ["logs:CreateLogGroup", "logs:CreateLogStream", "logs:PutLogEvents", "logs:Describe*"]
    resources = ["arn:aws:logs:${var.aws_region}:${var.aws_acct_id}:*"]
  }
}

data "aws_iam_policy_document" "jakesky_assume_role_policy_document" {
  statement {
    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
    actions = ["sts:AssumeRole"]
  }
}

resource "aws_iam_policy" "jakesky_role_policy" {
  name   = "jakesky.lambda"
  policy = data.aws_iam_policy_document.jakesky_role_policy_document.json
}

resource "aws_iam_role" "jakesky_role" {
  name               = "jakesky.lambda"
  assume_role_policy = data.aws_iam_policy_document.jakesky_assume_role_policy_document.json
}

resource "aws_iam_role_policy_attachment" "jakesky_role_attachment" {
  role       = aws_iam_role.jakesky_role.name
  policy_arn = aws_iam_policy.jakesky_role_policy.arn
}

resource "aws_lambda_function" "jakesky" {
  function_name = "jakesky"
  s3_bucket     = var.code_bucket
  s3_key        = "jakesky.zip"
  role          = aws_iam_role.jakesky_role.arn
  architectures = ["x86_64"]
  runtime       = "provided.al2023"
  handler       = "ignored"
  publish       = "false"
  description   = "Retrieve local weather for commutes and lunchtime"
  timeout       = 5
  memory_size   = 128

  environment {
    variables = {
      JAKESKY_API_KEY      = var.jakesky_api_key
      JAKESKY_GEOCODIO_KEY = var.jakesky_geocodio_key
      JAKESKY_LATITUDE     = var.jakesky_latitude
      JAKESKY_LONGITUDE    = var.jakesky_longitude
    }
  }
}

resource "aws_cloudwatch_log_group" "jakesky_logs" {
  name              = "/aws/lambda/jakesky"
  retention_in_days = "7"
}

resource "aws_lambda_permission" "allow_alexa" {
  statement_id       = "AllowExecutionFromAlexa"
  action             = "lambda:InvokeFunction"
  function_name      = aws_lambda_function.jakesky.function_name
  principal          = "alexa-appkit.amazon.com"
  event_source_token = var.jakesky_skill_id
}

resource "aws_iam_openid_connect_provider" "github" {
  url = "https://token.actions.githubusercontent.com"

  client_id_list = ["sts.amazonaws.com"]

  thumbprint_list = ["6938fd4d98bab03faadb97b34396831e3780aea1"]
}

resource "aws_s3_bucket" "code" {
  bucket = var.code_bucket
}

data "aws_iam_policy_document" "github_deploy" {
  statement {
    actions = ["s3:PutObject"]
    resources = ["${aws_s3_bucket.code.arn}/jakesky.zip"]
  }
}

resource "aws_iam_policy" "github" {
  name   = "jakesky.github"
  policy = data.aws_iam_policy_document.github_deploy.json
}

resource "aws_iam_role" "github" {
  name = "jakesky.github"

  assume_role_policy = jsonencode({
    Version = "2012-10-17",
    Statement = [
      {
        Effect = "Allow",
        Principal = {
          Federated = aws_iam_openid_connect_provider.github.arn
        },
        Action = "sts:AssumeRoleWithWebIdentity",
        Condition = {
          StringEquals = {
            "token.actions.githubusercontent.com:aud" : "sts.amazonaws.com"
          }
          StringLike = {
            "token.actions.githubusercontent.com:sub" : "repo:jluszcz/JakeSky-rs:*"
          },
        }
      }
    ]
  })
}
