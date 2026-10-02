from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.claim_mapping import ClaimMapping
  from ..models.group_role import GroupRole
  from ..models.required_claim import RequiredClaim





T = TypeVar("T", bound="ProviderSettings")



@_attrs_define
class ProviderSettings:
    """ A provider's configuration, kept as one JSON document.

        Attributes:
            issuer (str): The issuer, as its ID tokens name it; discovery is read from
                `<issuer>/.well-known/openid-configuration`.
            client_id (str):
            scopes (list[str] | Unset): Scopes asked for besides `openid`.
            claims (ClaimMapping | Unset): Which claims become the user's names.
            group_roles (list[GroupRole] | Unset): Groups the provider asserts, and the role each gives. The most
                privileged role a user's groups give wins, and is set at every
                sign-in; with no mapping, a user's role is left as an admin set it.
            jit (bool | Unset): Create a user the first time someone the provider vouches for
                signs in, with no FCP account yet.
            link_by_email (bool | Unset): Link an existing user whose email is the one the provider asserts
                as verified, on their first sign-in.
            email_verified_claim (str | Unset): The claim that says the email is verified: `email_verified`, the
                standard one; `xms_edov` for Entra ID, whose tokens carry no
                `email_verified`.
            required_claims (list[RequiredClaim] | Unset): Claims a token must carry to sign anyone in: Google's `hd` (the
                Workspace domain), so a personal Google account is turned away;
                Entra's `tid`. A claim that is a list must contain the value.
     """

    issuer: str
    client_id: str
    scopes: list[str] | Unset = UNSET
    claims: ClaimMapping | Unset = UNSET
    group_roles: list[GroupRole] | Unset = UNSET
    jit: bool | Unset = UNSET
    link_by_email: bool | Unset = UNSET
    email_verified_claim: str | Unset = UNSET
    required_claims: list[RequiredClaim] | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        from ..models.claim_mapping import ClaimMapping # noqa: PLC0415
        from ..models.group_role import GroupRole # noqa: PLC0415
        from ..models.required_claim import RequiredClaim # noqa: PLC0415
        issuer = self.issuer

        client_id = self.client_id

        scopes: list[str] | Unset = UNSET
        if not isinstance(self.scopes, Unset):
            scopes = self.scopes



        claims: dict[str, Any] | Unset = UNSET
        if not isinstance(self.claims, Unset):
            claims = self.claims.to_dict()

        group_roles: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.group_roles, Unset):
            group_roles = []
            for group_roles_item_data in self.group_roles:
                group_roles_item = group_roles_item_data.to_dict()
                group_roles.append(group_roles_item)



        jit = self.jit

        link_by_email = self.link_by_email

        email_verified_claim = self.email_verified_claim

        required_claims: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.required_claims, Unset):
            required_claims = []
            for required_claims_item_data in self.required_claims:
                required_claims_item = required_claims_item_data.to_dict()
                required_claims.append(required_claims_item)




        field_dict: dict[str, Any] = {}

        field_dict.update({
            "issuer": issuer,
            "client_id": client_id,
        })
        if scopes is not UNSET:
            field_dict["scopes"] = scopes
        if claims is not UNSET:
            field_dict["claims"] = claims
        if group_roles is not UNSET:
            field_dict["group_roles"] = group_roles
        if jit is not UNSET:
            field_dict["jit"] = jit
        if link_by_email is not UNSET:
            field_dict["link_by_email"] = link_by_email
        if email_verified_claim is not UNSET:
            field_dict["email_verified_claim"] = email_verified_claim
        if required_claims is not UNSET:
            field_dict["required_claims"] = required_claims

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.claim_mapping import ClaimMapping # noqa: PLC0415
        from ..models.group_role import GroupRole # noqa: PLC0415
        from ..models.required_claim import RequiredClaim # noqa: PLC0415
        d = dict(src_dict)
        issuer = d.pop("issuer")

        client_id = d.pop("client_id")

        scopes = cast(list[str], d.pop("scopes", UNSET))


        _claims = d.pop("claims", UNSET)
        claims: ClaimMapping | Unset
        if isinstance(_claims,  Unset):
            claims = UNSET
        else:
            claims = ClaimMapping.from_dict(_claims)




        _group_roles = d.pop("group_roles", UNSET)
        group_roles: list[GroupRole] | Unset = UNSET
        if _group_roles is not UNSET:
            group_roles = []
            for group_roles_item_data in _group_roles:
                group_roles_item = GroupRole.from_dict(group_roles_item_data)



                group_roles.append(group_roles_item)


        jit = d.pop("jit", UNSET)

        link_by_email = d.pop("link_by_email", UNSET)

        email_verified_claim = d.pop("email_verified_claim", UNSET)

        _required_claims = d.pop("required_claims", UNSET)
        required_claims: list[RequiredClaim] | Unset = UNSET
        if _required_claims is not UNSET:
            required_claims = []
            for required_claims_item_data in _required_claims:
                required_claims_item = RequiredClaim.from_dict(required_claims_item_data)



                required_claims.append(required_claims_item)


        provider_settings = cls(
            issuer=issuer,
            client_id=client_id,
            scopes=scopes,
            claims=claims,
            group_roles=group_roles,
            jit=jit,
            link_by_email=link_by_email,
            email_verified_claim=email_verified_claim,
            required_claims=required_claims,
        )

        return provider_settings

