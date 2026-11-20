from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.join_token_response import JoinTokenResponse
  from ..models.registrant_response import RegistrantResponse





T = TypeVar("T", bound="RegisteredResponse")



@_attrs_define
class RegisteredResponse:
    """ A registration and the token minted for it.

        Attributes:
            registrant (RegistrantResponse): A registration as the API shows it.
            token (JoinTokenResponse): A minted token.
     """

    registrant: RegistrantResponse
    token: JoinTokenResponse
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.join_token_response import JoinTokenResponse # noqa: PLC0415
        from ..models.registrant_response import RegistrantResponse # noqa: PLC0415
        registrant = self.registrant.to_dict()

        token = self.token.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "registrant": registrant,
            "token": token,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.join_token_response import JoinTokenResponse # noqa: PLC0415
        from ..models.registrant_response import RegistrantResponse # noqa: PLC0415
        d = dict(src_dict)
        registrant = RegistrantResponse.from_dict(d.pop("registrant"))




        token = JoinTokenResponse.from_dict(d.pop("token"))




        registered_response = cls(
            registrant=registrant,
            token=token,
        )


        registered_response.additional_properties = d
        return registered_response

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
