from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.api_token_response import ApiTokenResponse





T = TypeVar("T", bound="CreateApiTokenResponse")



@_attrs_define
class CreateApiTokenResponse:
    """ Response body after creating a token (includes the one-time secret).

        Attributes:
            token (ApiTokenResponse): Response model for an API token (never includes a hash).
            secret (str):
     """

    token: ApiTokenResponse
    secret: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.api_token_response import ApiTokenResponse # noqa: PLC0415
        token = self.token.to_dict()

        secret = self.secret


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "token": token,
            "secret": secret,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.api_token_response import ApiTokenResponse # noqa: PLC0415
        d = dict(src_dict)
        token = ApiTokenResponse.from_dict(d.pop("token"))




        secret = d.pop("secret")

        create_api_token_response = cls(
            token=token,
            secret=secret,
        )


        create_api_token_response.additional_properties = d
        return create_api_token_response

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
