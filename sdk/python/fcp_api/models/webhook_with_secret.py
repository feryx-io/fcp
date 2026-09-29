from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.webhook_response import WebhookResponse





T = TypeVar("T", bound="WebhookWithSecret")



@_attrs_define
class WebhookWithSecret:
    """ A webhook just made or rotated, with its secret: the only time the
    secret is shown.

        Attributes:
            webhook (WebhookResponse): A webhook, without its secret.
            secret (str): `whsec_…`: verify each delivery's `X-FCP-Signature` with it.
     """

    webhook: WebhookResponse
    secret: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.webhook_response import WebhookResponse # noqa: PLC0415
        webhook = self.webhook.to_dict()

        secret = self.secret


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "webhook": webhook,
            "secret": secret,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.webhook_response import WebhookResponse # noqa: PLC0415
        d = dict(src_dict)
        webhook = WebhookResponse.from_dict(d.pop("webhook"))




        secret = d.pop("secret")

        webhook_with_secret = cls(
            webhook=webhook,
            secret=secret,
        )


        webhook_with_secret.additional_properties = d
        return webhook_with_secret

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
